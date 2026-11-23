//! A process that faults says so on stderr, and with `SK_CRASH_DUMPS=<dir>` leaves a minidump there.
//!
//! A host started by a test runner inherits the runner's error mode, which turns Windows Error
//! Reporting off: without this, a crash leaves nothing but an exit code.

#[cfg(target_os = "windows")]
mod imp {
    use std::fs::File;
    use std::io::Write;
    use std::mem::ManuallyDrop;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        MiniDumpWithThreadInfo, MiniDumpWriteDump, SetUnhandledExceptionFilter,
        EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER,
        MINIDUMP_EXCEPTION_INFORMATION,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId,
    };

    struct State {
        stderr: usize,
        /// NUL-terminated, and the same path as text for the line that names it.
        dump: Option<(Vec<u16>, String)>,
    }

    static STATE: OnceLock<State> = OnceLock::new();
    static PREVIOUS: OnceLock<LPTOP_LEVEL_EXCEPTION_FILTER> = OnceLock::new();

    /// Formatted into a stack buffer: the heap may be what broke.
    fn line(out: &mut [u8; 200], code: u32, address: usize, thread: u32) -> usize {
        let mut n = 0;
        let mut put = |bytes: &[u8]| {
            let take = bytes.len().min(out.len() - n);
            out[n..n + take].copy_from_slice(&bytes[..take]);
            n += take;
        };
        fn hex(v: u64, digits: usize) -> [u8; 16] {
            let mut b = [b'0'; 16];
            for i in 0..digits {
                b[digits - 1 - i] = b"0123456789ABCDEF"[((v >> (i * 4)) & 0xF) as usize];
            }
            b
        }
        fn dec(mut v: u32) -> ([u8; 10], usize) {
            let mut b = [0u8; 10];
            let mut i = 10;
            loop {
                i -= 1;
                b[i] = b'0' + (v % 10) as u8;
                v /= 10;
                if v == 0 {
                    break;
                }
            }
            (b, i)
        }
        put(b"[crash] unhandled exception 0x");
        put(&hex(code as u64, 8)[..8]);
        put(b" at 0x");
        put(&hex(address as u64, 16));
        put(b" on thread ");
        let (d, i) = dec(thread);
        put(&d[i..]);
        put(b"\n");
        n
    }

    unsafe extern "system" fn report(info: *const EXCEPTION_POINTERS) -> i32 {
        let Some(state) = STATE.get() else {
            return EXCEPTION_CONTINUE_SEARCH;
        };
        // SAFETY: the system passes a valid record.
        let record = unsafe { &*(*info).ExceptionRecord };
        // SAFETY: a handle this process owns for its whole life; never closed here.
        let mut err =
            ManuallyDrop::new(unsafe { File::from_raw_handle(state.stderr as RawHandle) });
        let mut buf = [0u8; 200];
        let n = line(
            &mut buf,
            record.ExceptionCode as u32,
            record.ExceptionAddress as usize,
            unsafe { GetCurrentThreadId() },
        );
        let _ = err.write_all(&buf[..n]);
        if let Some((wide, text)) = &state.dump {
            // SAFETY: a NUL-terminated path; the dump borrows the faulting thread's own pointers.
            unsafe {
                let file = CreateFileW(
                    wide.as_ptr(),
                    GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    CREATE_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL,
                    std::ptr::null_mut(),
                );
                if file != INVALID_HANDLE_VALUE {
                    let exception = MINIDUMP_EXCEPTION_INFORMATION {
                        ThreadId: GetCurrentThreadId(),
                        ExceptionPointers: info as *mut EXCEPTION_POINTERS,
                        ClientPointers: 0,
                    };
                    let ok = MiniDumpWriteDump(
                        GetCurrentProcess(),
                        GetCurrentProcessId(),
                        file,
                        MiniDumpWithThreadInfo,
                        &exception,
                        std::ptr::null(),
                        std::ptr::null(),
                    );
                    CloseHandle(file);
                    if ok != 0 {
                        let _ = err.write_all(b"[crash] minidump: ");
                        let _ = err.write_all(text.as_bytes());
                        let _ = err.write_all(b"\n");
                    }
                }
            }
        }
        match PREVIOUS.get().copied().flatten() {
            // SAFETY: the filter that was installed before this one, with the same contract.
            Some(previous) => unsafe { previous(info) },
            None => EXCEPTION_CONTINUE_SEARCH,
        }
    }

    pub fn install() {
        let dump = std::env::var_os("SK_CRASH_DUMPS")
            .filter(|d| !d.is_empty())
            .and_then(|dir| {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).ok()?;
                let exe = std::env::current_exe().ok()?;
                let name = exe.file_stem()?.to_string_lossy().into_owned();
                let path = dir.join(format!("{name}.{}.dmp", std::process::id()));
                let wide = path.as_os_str().encode_wide().chain(Some(0)).collect();
                Some((wide, path.display().to_string()))
            });
        let stderr = std::io::stderr().as_raw_handle() as usize;
        if STATE.set(State { stderr, dump }).is_err() {
            return;
        }
        // SAFETY: installs a filter with the system's signature.
        let previous = unsafe { SetUnhandledExceptionFilter(Some(report)) };
        let _ = PREVIOUS.set(previous);
    }
}

#[cfg(target_os = "windows")]
pub use imp::install;

#[cfg(not(target_os = "windows"))]
pub fn install() {}
