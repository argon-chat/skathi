//! Single instance on Windows: a session-local mutex says whether an instance runs, a per-user named pipe
//! carries a second launch to it.
//!
//! ```text
//!   key  = fnv128(id ‖ storage folder name ‖ user SID)
//!   Local\vn-instance-<key>                 mutex: session-local, gone when the holder dies
//!   \\.\pipe\vn-instance-<key>-<session>    FIRST_PIPE_INSTANCE, REJECT_REMOTE_CLIENTS,
//!                                           MESSAGE mode, DACL = this user + SYSTEM
//! ```
//!
//! Both ends verify the other: the server checks the client's token user, the client the server's token
//! user AND image, so a process squatting the pipe name gets nothing. One message per connection, strict
//! JSON, acked with one byte: `1` delivered, `2` closing (the second launch waits for it and becomes the instance).

use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS,
    ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ,
    GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, GetFileInformationByHandle, ReadFile, WriteFile,
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
    SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    GetNamedPipeServerProcessId, SetNamedPipeHandleState, WaitNamedPipeW, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindowAsync,
    SW_RESTORE, SW_SHOW,
};

use super::{Forward, CLOSING_PATIENCE_MS, FORWARD_TIMEOUT_MS, MAX_MESSAGE};

pub(super) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn os_error(code: u32) -> std::io::Error {
    std::io::Error::from_raw_os_error(code as i32)
}

#[derive(Debug)]
pub enum Claim {
    Primary,
    Unavailable(String),
    Forwarded,
    NoAnswer,
    StillClosing,
    Impostor(String),
}

struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
unsafe impl Send for Owned {}

pub fn fnv128(parts: &[&[u8]]) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013B;
    let mut h = OFFSET;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            h ^= 0xff;
            h = h.wrapping_mul(PRIME);
        }
        for &b in *part {
            h ^= u128::from(b);
            h = h.wrapping_mul(PRIME);
        }
    }
    h
}

pub fn instance_key(id: &str, storage: &str, sid: &str) -> String {
    format!(
        "{:032x}",
        fnv128(&[id.as_bytes(), storage.as_bytes(), sid.as_bytes()])
    )
}

fn token_user_sid(process: HANDLE) -> Option<String> {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return None;
        }
        let token = Owned(token);
        let mut len = 0u32;
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut len);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        if GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) == 0 {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            return None;
        }
        let units = (0..).take_while(|&i| *s.add(i) != 0).count();
        let out = String::from_utf16(std::slice::from_raw_parts(s, units)).ok();
        LocalFree(s.cast());
        out
    }
}

fn own_sid() -> Option<String> {
    token_user_sid(unsafe { GetCurrentProcess() })
}

fn open_peer(pid: u32) -> Option<Owned> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    (!h.is_null()).then_some(Owned(h))
}

fn image_path(process: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 32_768];
    let mut len = buf.len() as u32;
    if unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
    } == 0
    {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

fn file_identity(path: &str) -> Option<(u32, u64)> {
    unsafe {
        let h = CreateFileW(
            wide(path).as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let h = Owned(h);
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if GetFileInformationByHandle(h.0, &mut info) == 0 {
            return None;
        }
        Some((
            info.dwVolumeSerialNumber,
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
}

fn verify_server(pid: u32, sid: &str) -> Result<(), String> {
    let peer = open_peer(pid).ok_or_else(|| format!("cannot open pid {pid}"))?;
    let peer_sid = token_user_sid(peer.0).ok_or_else(|| format!("cannot read pid {pid}'s user"))?;
    if peer_sid != sid {
        return Err(format!("pid {pid} runs as {peer_sid}"));
    }
    let theirs = image_path(peer.0).ok_or_else(|| format!("cannot read pid {pid}'s image"))?;
    let ours = image_path(unsafe { GetCurrentProcess() }).ok_or("cannot read our own image")?;
    if theirs.eq_ignore_ascii_case(&ours) {
        return Ok(());
    }
    match (file_identity(&theirs), file_identity(&ours)) {
        (Some(a), Some(b)) if a == b => Ok(()),
        _ => Err(format!("pid {pid} is {theirs}")),
    }
}

/// The pipe's DACL: this user and SYSTEM, nobody else (the default lets Everyone read).
struct Dacl(PSECURITY_DESCRIPTOR);
impl Dacl {
    fn for_user(sid: &str) -> Option<Dacl> {
        let sddl = wide(&format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)"));
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        (ok != 0).then_some(Dacl(sd))
    }
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}
impl Drop for Dacl {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
unsafe impl Send for Dacl {}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

pub fn claim(id: &str, storage: &str, fwd: &Forward) -> Claim {
    claim_within(id, storage, fwd, Duration::from_millis(FORWARD_TIMEOUT_MS))
}

fn claim_within(id: &str, storage: &str, fwd: &Forward, patience: Duration) -> Claim {
    let Some(sid) = own_sid() else {
        return Claim::Unavailable("this process's user SID could not be read".into());
    };
    let key = instance_key(id, storage, &sid);
    let mut session = 0u32;
    unsafe {
        ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
    }
    let mutex_name = wide(&format!("Local\\vn-instance-{key}"));
    let pipe_name = format!(r"\\.\pipe\vn-instance-{key}-{session}");

    let mut deadline = Instant::now() + patience;
    let mut closing = false;
    loop {
        match take_mutex(&mutex_name) {
            Mutex::Ours => {
                return match serve(pipe_name, sid, deadline) {
                    Ok(()) => Claim::Primary,
                    Err(why) => {
                        eprintln!("[launch] the single-instance pipe could not be created: {why}");
                        Claim::Primary
                    }
                };
            }
            Mutex::Denied => {
                return Claim::Unavailable(
                    "the instance mutex exists and is not accessible to this user".into(),
                )
            }
            Mutex::Failed(why) => return Claim::Unavailable(why),
            Mutex::Taken => {}
        }
        match forward(&pipe_name, &sid, fwd, deadline) {
            Sent::Acked => return Claim::Forwarded,
            Sent::Impostor(why) => return Claim::Impostor(why),
            Sent::Closing if !closing || Instant::now() < deadline => {
                if !closing {
                    // Once: a closing instance ends itself within its own deadline, so waiting is bounded.
                    closing = true;
                    deadline =
                        deadline.max(Instant::now() + Duration::from_millis(CLOSING_PATIENCE_MS));
                }
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Sent::Gone if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            // A closing instance that dies mid-exchange breaks the pipe instead of removing it.
            Sent::NoAnswer if closing && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Sent::Closing => return Claim::StillClosing,
            Sent::Gone | Sent::NoAnswer if closing => return Claim::StillClosing,
            Sent::Gone | Sent::NoAnswer => return Claim::NoAnswer,
        }
    }
}

enum Mutex {
    Ours,
    Taken,
    Denied,
    Failed(String),
}

fn take_mutex(name: &[u16]) -> Mutex {
    let h = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
    let e = last_error();
    if h.is_null() {
        return if e == ERROR_ACCESS_DENIED {
            Mutex::Denied
        } else {
            Mutex::Failed(format!("CreateMutexW: {}", os_error(e)))
        };
    }
    if e == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(h);
        }
        Mutex::Taken
    } else {
        // Held for the life of the process; the kernel drops it when we die.
        std::mem::forget(Owned(h));
        Mutex::Ours
    }
}

enum Sent {
    Acked,
    Closing,
    Gone,
    NoAnswer,
    Impostor(String),
}

const ACK_DELIVERED: u8 = 1;
const ACK_CLOSING: u8 = 2;

fn forward(pipe_name: &str, sid: &str, fwd: &Forward, deadline: Instant) -> Sent {
    let name = wide(pipe_name);
    let pipe = loop {
        let opened = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                // Identification only: the server may ask who we are, never act as us.
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                null_mut(),
            )
        };
        if opened != INVALID_HANDLE_VALUE {
            break Owned(opened);
        }
        let e = last_error();
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Sent::NoAnswer;
        }
        if e == ERROR_PIPE_BUSY {
            unsafe {
                WaitNamedPipeW(
                    name.as_ptr(),
                    remaining.as_millis().min(u128::from(u32::MAX)) as u32,
                );
            }
            continue;
        }
        if e == ERROR_FILE_NOT_FOUND {
            return Sent::Gone;
        }
        return Sent::NoAnswer;
    };
    unsafe {
        let mode = PIPE_READMODE_MESSAGE;
        SetNamedPipeHandleState(pipe.0, &mode, null(), null());
    }
    let mut server_pid = 0u32;
    if unsafe { GetNamedPipeServerProcessId(pipe.0, &mut server_pid) } == 0 {
        return Sent::Impostor("the pipe server's pid could not be read".into());
    }
    if let Err(why) = verify_server(server_pid, sid) {
        return Sent::Impostor(why);
    }
    // The secondary was just started by the user, so it may hand the foreground on; a quiet launch does not.
    if fwd.raise {
        unsafe {
            AllowSetForegroundWindow(server_pid);
        }
    }
    let bytes = fwd.encode();
    if bytes.len() > MAX_MESSAGE {
        eprintln!("[launch] this launch's arguments are over {MAX_MESSAGE} bytes; not forwarded");
        return Sent::NoAnswer;
    }
    let mut written = 0u32;
    let wrote = unsafe {
        WriteFile(
            pipe.0,
            bytes.as_ptr(),
            bytes.len() as u32,
            &mut written,
            null_mut(),
        )
    };
    if wrote == 0 || written as usize != bytes.len() {
        return Sent::NoAnswer;
    }
    // The ack has a deadline: a wedged primary that accepted must not hold this process forever.
    let (tx, rx) = mpsc::channel();
    let raw = pipe.0 as usize;
    std::thread::spawn(move || {
        let mut ack = [0u8; 1];
        let mut n = 0u32;
        let ok = unsafe {
            ReadFile(
                raw as *mut c_void,
                ack.as_mut_ptr(),
                ack.len() as u32,
                &mut n,
                null_mut(),
            )
        } != 0;
        let _ = tx.send(if ok && n == 1 { ack[0] } else { 0 });
    });
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(250));
    match rx.recv_timeout(remaining) {
        Ok(ACK_DELIVERED) => Sent::Acked,
        Ok(ACK_CLOSING) => Sent::Closing,
        // Leaking the handle on a timeout is deliberate: the reader thread still owns a read on it.
        Err(_) => {
            std::mem::forget(pipe);
            Sent::NoAnswer
        }
        Ok(_) => Sent::NoAnswer,
    }
}

const PIPE_BUFFER: u32 = 64 * 1024;

fn create_instance(name: &[u16], dacl: &Dacl, first: bool) -> Result<Owned, String> {
    let sa = dacl.attributes();
    let open_mode = if first {
        PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE
    } else {
        PIPE_ACCESS_DUPLEX
    };
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            &sa,
        )
    };
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        let e = last_error();
        return Err(if e == ERROR_ACCESS_DENIED {
            "the pipe name is already taken by another process".to_string()
        } else {
            format!("CreateNamedPipeW failed ({e})")
        });
    }
    Ok(Owned(h))
}

/// Creates the first pipe instance synchronously (so a failure is reported), then hands accepting to the
/// `vn-instance` thread. A taken name is retried for up to a second: the previous instance's pipe may linger.
fn serve(pipe_name: String, sid: String, deadline: Instant) -> Result<(), String> {
    let dacl = Dacl::for_user(&sid).ok_or("the pipe's security descriptor could not be built")?;
    let name = wide(&pipe_name);
    let give_up = deadline.min(Instant::now() + Duration::from_secs(1));
    let first = loop {
        match create_instance(&name, &dacl, true) {
            Ok(p) => break p,
            Err(_) if Instant::now() < give_up => std::thread::sleep(Duration::from_millis(50)),
            Err(why) => return Err(why),
        }
    };
    std::thread::Builder::new()
        .name("vn-instance".into())
        .spawn(move || {
            let mut pending = Some(first);
            loop {
                let pipe = match pending.take() {
                    Some(p) => p,
                    None => match create_instance(&name, &dacl, false) {
                        Ok(p) => p,
                        Err(why) => {
                            eprintln!("[launch] the single-instance pipe stopped accepting: {why}");
                            return;
                        }
                    },
                };
                let connected = unsafe { ConnectNamedPipe(pipe.0, null_mut()) } != 0
                    || last_error() == ERROR_PIPE_CONNECTED;
                if !connected {
                    continue;
                }
                // The next instance exists BEFORE this client is handled, so a third launch never finds the name missing.
                match create_instance(&name, &dacl, false) {
                    Ok(next) => pending = Some(next),
                    Err(why) => {
                        eprintln!("[launch] could not create the next pipe instance: {why}")
                    }
                }
                let sid = sid.clone();
                let _ = std::thread::Builder::new()
                    .name("vn-instance-client".into())
                    .spawn(move || handle(pipe, &sid));
            }
        })
        .map_err(|e| format!("spawning the pipe thread: {e}"))?;
    Ok(())
}

fn handle(pipe: Owned, sid: &str) {
    let mut client_pid = 0u32;
    let same_user = unsafe { GetNamedPipeClientProcessId(pipe.0, &mut client_pid) } != 0
        && open_peer(client_pid)
            .and_then(|p| token_user_sid(p.0))
            .as_deref()
            == Some(sid);
    if !same_user {
        eprintln!(
            "[launch] refused a single-instance message from pid {client_pid}: not this user"
        );
        unsafe {
            DisconnectNamedPipe(pipe.0);
        }
        return;
    }
    let msg = match read_message(&pipe) {
        Ok(bytes) => Forward::decode(&bytes),
        Err(why) => Err(why),
    };
    let msg = match msg {
        Ok(m) => m,
        Err(why) => {
            eprintln!("[launch] refused a single-instance message from pid {client_pid}: {why}");
            unsafe {
                DisconnectNamedPipe(pipe.0);
            }
            return;
        }
    };
    let closing = super::is_closing();
    unsafe {
        let mut n = 0u32;
        let ack = [if closing { ACK_CLOSING } else { ACK_DELIVERED }];
        WriteFile(pipe.0, ack.as_ptr(), 1, &mut n, null_mut());
        FlushFileBuffers(pipe.0);
        DisconnectNamedPipe(pipe.0);
    }
    drop(pipe);
    if closing {
        // stderr, not the log ring: a closing instance drains the ring no more.
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("[launch] a second launch arrived while this instance is closing; it starts the app once this one has ended");
        }
        return;
    }
    super::deliver(msg);
}

fn read_message(pipe: &Owned) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; PIPE_BUFFER as usize];
    loop {
        let mut n = 0u32;
        let ok = unsafe {
            ReadFile(
                pipe.0,
                chunk.as_mut_ptr(),
                chunk.len() as u32,
                &mut n,
                null_mut(),
            )
        } != 0;
        let e = last_error();
        out.extend_from_slice(&chunk[..n as usize]);
        if out.len() > MAX_MESSAGE {
            return Err(format!("message over the {MAX_MESSAGE}-byte cap"));
        }
        match e {
            _ if ok => return Ok(out),
            ERROR_MORE_DATA => continue,
            e => return Err(format!("read failed: {}", os_error(e))),
        }
    }
}

pub fn raise(hwnd: isize) {
    let hwnd = hwnd as *mut c_void;
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindowAsync(hwnd, SW_RESTORE);
        } else if IsWindowVisible(hwnd) == 0 {
            ShowWindowAsync(hwnd, SW_SHOW);
        }
        SetForegroundWindow(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_deterministic_and_separates_its_parts() {
        let a = instance_key("gl.argon.notes", "notes", "S-1-5-21-1");
        assert_eq!(a, instance_key("gl.argon.notes", "notes", "S-1-5-21-1"));
        assert_eq!(a.len(), 32);
        assert_ne!(
            a,
            instance_key("gl.argon.notes", "notes", "S-1-5-21-2"),
            "per user"
        );
        assert_ne!(
            a,
            instance_key("gl.argon.other", "notes", "S-1-5-21-1"),
            "per app"
        );
        assert_ne!(
            a,
            instance_key("gl.argon.notes", "notes-2", "S-1-5-21-1"),
            "per storage folder"
        );
        assert_ne!(fnv128(&[b"ab", b"c"]), fnv128(&[b"a", b"bc"]));
    }

    #[test]
    fn our_own_sid_and_image_verify() {
        let sid = own_sid().expect("a SID");
        assert!(sid.starts_with("S-1-"), "{sid}");
        verify_server(unsafe { GetCurrentProcessId() }, &sid).expect("we are ourselves");
        assert!(
            verify_server(unsafe { GetCurrentProcessId() }, "S-1-0-0").is_err(),
            "another user"
        );
    }

    #[test]
    fn a_process_with_another_image_is_an_impostor() {
        let parent = std::process::Command::new("cmd")
            .args(["/c", "exit 0"])
            .spawn();
        if let Ok(mut child) = parent {
            let pid = child.id();
            let sid = own_sid().unwrap();
            let r = verify_server(pid, &sid);
            let _ = child.wait();
            assert!(r.is_err());
        }
    }

    #[test]
    fn a_holder_that_never_answers_is_no_answer() {
        let id = format!("sk-desktop-wedged-{}", std::process::id());
        let sid = own_sid().unwrap();
        let key = instance_key(&id, "wedged", &sid);
        assert!(matches!(
            take_mutex(&wide(&format!("Local\\vn-instance-{key}"))),
            Mutex::Ours
        ));
        let fwd = Forward {
            argv: vec![],
            args: vec![],
            cwd: String::new(),
            url: None,
            raise: false,
        };
        let t0 = Instant::now();
        assert!(matches!(
            claim_within(&id, "wedged", &fwd, Duration::from_millis(400)),
            Claim::NoAnswer
        ));
        assert!(
            t0.elapsed() >= Duration::from_millis(350),
            "it waited for its deadline"
        );
    }

    #[test]
    fn a_second_claim_forwards_to_the_first() {
        let id = format!("sk-desktop-test-{}", std::process::id());
        let fwd = Forward {
            argv: vec!["x.exe".into(), "--y".into()],
            args: vec!["--y".into()],
            cwd: "C:\\".into(),
            url: Some("vntest://a?b=\"c d\"".into()),
            raise: false,
        };
        assert!(matches!(claim(&id, "forward", &fwd), Claim::Primary));
        assert!(matches!(claim(&id, "forward", &fwd), Claim::Forwarded));
        let deadline = Instant::now() + Duration::from_secs(5);
        let found = loop {
            let hit = crate::take_emitted()
                .into_iter()
                .find(|p| p.contains("vntest://a?b="));
            if let Some(p) = hit {
                break p;
            }
            if Instant::now() > deadline {
                panic!("the forwarded launch never arrived");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let v: serde_json::Value = serde_json::from_str(&found).unwrap();
        assert_eq!(v["kind"], "launch");
        assert_eq!(v["source"], "second-instance");
        assert_eq!(v["url"], "vntest://a?b=\"c d\"");
        assert_eq!(v["argv"][1], "--y");
    }
}
