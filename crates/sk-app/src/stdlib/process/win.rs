//! CreateProcessW with the program as `lpApplicationName` (no search), only the three stdio handles
//! inherited, and the child in a job object of its own that kills its tree when closed.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_SAME_ACCESS, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFinalPathNameByHandleW, FILE_ATTRIBUTE_NORMAL, FILE_NAME_NORMALIZED,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, VOLUME_NAME_DOS,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, PROCESS_INFORMATION, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use super::child::{Child, Exit, Request, Signal, Spawned, State, Stdio};
use super::resolve;

/// The command line CreateProcessW takes, in UTF-16 units with its NUL.
const COMMAND_LINE_LIMIT: usize = 32_767;

pub(super) struct Handle {
    process: OwnedHandle,
    job: OwnedHandle,
}

// SAFETY: process and job handles may be used from any thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn last_error() -> io::Error {
    io::Error::last_os_error()
}

fn owned(h: HANDLE) -> io::Result<OwnedHandle> {
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return Err(last_error());
    }
    // SAFETY: a fresh handle this function now owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(h as RawHandle) })
}

fn inheritable(h: &OwnedHandle) -> io::Result<OwnedHandle> {
    let mut dup: HANDLE = null_mut();
    // SAFETY: duplicating a handle of this process into this process.
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            h.as_raw_handle() as HANDLE,
            GetCurrentProcess(),
            &mut dup,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    owned(dup)
}

/// Opens the program so that it cannot be written, renamed or deleted until the process is
/// created, and checks that the file opened is the one matched.
fn hold(program: &Path) -> io::Result<File> {
    let name = wide(program.as_os_str());
    // SAFETY: a NUL-terminated path; no security attributes.
    let h = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ,
            null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    let file = File::from(owned(h)?);
    let mut buf = vec![0u16; 1024];
    // SAFETY: the buffer is as long as said.
    let n = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle() as HANDLE,
            buf.as_mut_ptr(),
            buf.len() as u32,
            FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
        )
    } as usize;
    if n == 0 || n >= buf.len() {
        return Err(last_error());
    }
    let opened = resolve::plain(String::from_utf16_lossy(&buf[..n]).into());
    if !resolve::same_path(&opened, program) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the program changed while it was being started",
        ));
    }
    let mut magic = [0u8; 2];
    let mut reader = &file;
    if reader.read_exact(&mut magic).is_err() || &magic != b"MZ" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the program is not a Windows executable",
        ));
    }
    Ok(file)
}

fn null_device() -> io::Result<OwnedHandle> {
    let name = wide(OsStr::new("NUL"));
    // SAFETY: a NUL-terminated name; no security attributes.
    owned(unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    })
}

/// (the child's end, the parent's end)
fn pipe(child_reads: bool) -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut r, mut w): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: two out-pointers; the handles are not inheritable.
    if unsafe { CreatePipe(&mut r, &mut w, null(), 0) } == 0 {
        return Err(last_error());
    }
    let (r, w) = (owned(r)?, owned(w)?);
    Ok(if child_reads { (r, w) } else { (w, r) })
}

/// The quoting the MSVC runtime and `CommandLineToArgvW` undo, so the child's argv is `arg` exactly.
pub(super) fn push_arg(out: &mut Vec<u16>, arg: &str) {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        out.extend(arg.encode_utf16());
        return;
    }
    out.push('"' as u16);
    let mut backslashes = 0usize;
    for c in arg.encode_utf16() {
        if c == '\\' as u16 {
            backslashes += 1;
            continue;
        }
        let extra = if c == '"' as u16 { backslashes + 1 } else { 0 };
        out.extend(std::iter::repeat_n('\\' as u16, backslashes + extra));
        out.push(c);
        backslashes = 0;
    }
    out.extend(std::iter::repeat_n('\\' as u16, backslashes * 2));
    out.push('"' as u16);
}

pub(super) fn command_line(program: &Path, args: &[String]) -> io::Result<Vec<u16>> {
    let mut line = vec!['"' as u16];
    line.extend(program.as_os_str().encode_wide());
    line.push('"' as u16);
    for arg in args {
        line.push(' ' as u16);
        push_arg(&mut line, arg);
    }
    if line.len() >= COMMAND_LINE_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "the command line is {} characters, and Windows takes at most {}",
                line.len(),
                COMMAND_LINE_LIMIT - 1
            ),
        ));
    }
    line.push(0);
    Ok(line)
}

fn environment_block(env: &[(std::ffi::OsString, std::ffi::OsString)]) -> Vec<u16> {
    let mut sorted: Vec<_> = env.iter().collect();
    sorted.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (k, v) in sorted {
        block.extend(k.encode_wide());
        block.push('=' as u16);
        block.extend(v.encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

fn job() -> io::Result<OwnedHandle> {
    // SAFETY: an anonymous job; no security attributes.
    let job = owned(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: the structure the information class names, with its size.
    let ok = unsafe {
        SetInformationJobObject(
            job.as_raw_handle() as HANDLE,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    Ok(job)
}

struct Attributes(Vec<u8>);

impl Attributes {
    fn handle_list(handles: &[HANDLE]) -> io::Result<Attributes> {
        let mut size = 0usize;
        // SAFETY: a size query; it fails with ERROR_INSUFFICIENT_BUFFER by design.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size) };
        let mut list = Attributes(vec![0u8; size]);
        // SAFETY: a buffer of the size asked for.
        if unsafe { InitializeProcThreadAttributeList(list.ptr(), 1, 0, &mut size) } == 0 {
            list.0.clear();
            return Err(last_error());
        }
        // SAFETY: `handles` outlives the list, which is used only for the CreateProcessW below.
        let ok = unsafe {
            UpdateProcThreadAttribute(
                list.ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                null_mut(),
                null(),
            )
        };
        if ok == 0 {
            return Err(last_error());
        }
        Ok(list)
    }

    fn ptr(&mut self) -> *mut core::ffi::c_void {
        self.0.as_mut_ptr().cast()
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        if !self.0.is_empty() {
            // SAFETY: initialized above.
            unsafe { DeleteProcThreadAttributeList(self.ptr()) };
        }
    }
}

pub(super) fn spawn(request: &Request<'_>) -> io::Result<Spawned> {
    let held = hold(request.program)?;
    let mut nul = None;
    let mut ends = |mode: Stdio, child_reads: bool| -> io::Result<(OwnedHandle, Option<File>)> {
        match mode {
            Stdio::Pipe => {
                let (child, parent) = pipe(child_reads)?;
                Ok((inheritable(&child)?, Some(File::from(parent))))
            }
            Stdio::Null => {
                if nul.is_none() {
                    nul = Some(null_device()?);
                }
                Ok((inheritable(nul.as_ref().unwrap())?, None))
            }
        }
    };
    let (child_in, stdin) = ends(request.stdin, true)?;
    let (child_out, stdout) = ends(request.stdout, false)?;
    let (child_err, stderr) = ends(request.stderr, false)?;
    let inherited: Vec<HANDLE> = [&child_in, &child_out, &child_err]
        .iter()
        .map(|h| h.as_raw_handle() as HANDLE)
        .collect();
    let mut attributes = Attributes::handle_list(&inherited)?;

    let application = wide(request.program.as_os_str());
    let mut line = command_line(request.program, request.args)?;
    let env = environment_block(&request.env);
    let cwd = request.cwd.map(|c| wide(c.as_os_str()));
    let job = job()?;

    let mut info = STARTUPINFOEXW::default();
    info.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    info.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    info.StartupInfo.hStdInput = inherited[0];
    info.StartupInfo.hStdOutput = inherited[1];
    info.StartupInfo.hStdError = inherited[2];
    info.lpAttributeList = attributes.ptr();
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: every pointer is to a live NUL-terminated buffer or structure; inheritance is
    // limited to the three handles of the attribute list.
    let ok = unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED
                | CREATE_NO_WINDOW
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            env.as_ptr().cast(),
            cwd.as_ref().map_or(null(), |c| c.as_ptr()),
            &info.StartupInfo,
            &mut process,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    drop(held);
    let handle = owned(process.hProcess)?;
    let thread = owned(process.hThread)?;
    // SAFETY: both handles are live; the process has not run a single instruction yet.
    unsafe {
        if AssignProcessToJobObject(job.as_raw_handle() as HANDLE, process.hProcess) == 0 {
            let error = last_error();
            TerminateProcess(process.hProcess, 1);
            return Err(error);
        }
        ResumeThread(thread.as_raw_handle() as HANDLE);
    }
    drop(thread);
    Ok(Spawned {
        child: Arc::new(Child {
            pid: process.dwProcessId,
            os: Handle {
                process: handle,
                job,
            },
            state: std::sync::Mutex::new(State::default()),
        }),
        stdin,
        stdout,
        stderr,
    })
}

pub(super) fn has_exited(os: &Handle) -> bool {
    // SAFETY: a live process handle; a zero timeout only polls.
    unsafe { WaitForSingleObject(os.process.as_raw_handle() as HANDLE, 0) == WAIT_OBJECT_0 }
}

pub(super) fn kill(os: &Handle, _pid: u32, _signal: Signal) {
    end_tree(os);
}

pub(super) fn end_tree(os: &Handle) {
    // SAFETY: a live job handle.
    unsafe { TerminateJobObject(os.job.as_raw_handle() as HANDLE, 1) };
}

pub(super) fn wait(child: &Child) -> Exit {
    let process = child.os.process.as_raw_handle() as HANDLE;
    let mut code = 0u32;
    // SAFETY: a live process handle and an out-pointer.
    unsafe {
        WaitForSingleObject(process, INFINITE);
        GetExitCodeProcess(process, &mut code);
    }
    let mut state = child.state.lock().unwrap_or_else(|e| e.into_inner());
    state.exited = true;
    match state.killed {
        Some(signal) => Exit::Signal(signal.name().into()),
        None => Exit::Code(code as i64),
    }
}
