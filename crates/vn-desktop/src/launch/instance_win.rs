//! Single instance on Windows: a session-local mutex says whether an instance runs, a per-user named
//! pipe carries a second launch to it.
//!
//! ```text
//!   key  = fnv128(id ‖ storage folder name ‖ user SID)
//!   Local\vn-instance-<key>                 mutex: session-local, gone when the holder dies
//!   \\.\pipe\vn-instance-<key>-<session>    FIRST_PIPE_INSTANCE, REJECT_REMOTE_CLIENTS,
//!                                           MESSAGE mode, DACL = this user + SYSTEM
//! ```
//!
//! Both ends verify the other: the server checks the client's token user, the client the server's
//! token user AND image (path, or same file for a hard link), so a process squatting the pipe name
//! gets nothing. One message per connection, capped at [`super::MAX_MESSAGE`], strict JSON, acked
//! with one byte: `1` delivered, `2` closing (the second launch waits for it to exit and becomes
//! the instance).

use std::ffi::c_void;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND,
    ERROR_MORE_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, HWND,
    INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, GetFileInformationByHandle, ReadFile, WriteFile, BY_HANDLE_FILE_INFORMATION,
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    GetNamedPipeServerProcessId, SetNamedPipeHandleState, WaitNamedPipeW, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindowAsync, SW_RESTORE, SW_SHOW,
};

use super::{Forward, CLOSING_PATIENCE_MS, FORWARD_TIMEOUT_MS, MAX_MESSAGE};

#[derive(Debug)]
pub enum Claim {
    Primary,
    /// The rule could not be enforced (no SID, the mutex is not ours to open):
    /// run as a separate instance, said out loud by the caller.
    Unavailable(String),
    Forwarded,
    NoAnswer,
    /// The instance answered that it is closing, and had not exited by the
    /// deadline.
    StillClosing,
    /// The pipe's server is not this app.
    Impostor(String),
}

struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}
// The handles below are kernel objects used from the thread that owns them
// (or moved to exactly one other thread); nothing aliases them.
unsafe impl Send for Owned {}

/// FNV-1a, 128-bit: names a kernel object, protects nothing.
pub fn fnv128(parts: &[&[u8]]) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013B;
    let mut h = OFFSET;
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            // A separator no part can contain, so ("ab","c") ≠ ("a","bc").
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

/// The instance key: the id, the storage folder's name ([`super::storage_name`]) and the user.
pub fn instance_key(id: &str, storage: &str, sid: &str) -> String {
    format!("{:032x}", fnv128(&[id.as_bytes(), storage.as_bytes(), sid.as_bytes()]))
}

fn token_user_sid(process: HANDLE) -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let token = Owned(token);
        let mut len = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut len);
        if len == 0 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        GetTokenInformation(token.0, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len).ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s).ok()?;
        let out = s.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
        out
    }
}

fn own_sid() -> Option<String> {
    token_user_sid(unsafe { GetCurrentProcess() })
}

fn open_peer(pid: u32) -> Option<Owned> {
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok().map(Owned) }
}

fn image_path(process: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 32_768];
    let mut len = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).ok()? };
    Some(String::from_utf16_lossy(&buf[..len as usize]))
}

/// `(volume serial, file index)` — the same FILE, whatever name reached it.
fn file_identity(path: &str) -> Option<(u32, u64)> {
    unsafe {
        let h = CreateFileW(
            &HSTRING::from(path),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
        .ok()?;
        let h = Owned(h);
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        GetFileInformationByHandle(h.0, &mut info).ok()?;
        Some((info.dwVolumeSerialNumber, (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)))
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
    // A hard-linked copy (vn-test stages one per run) is the same exe under
    // another name.
    match (file_identity(&theirs), file_identity(&ours)) {
        (Some(a), Some(b)) if a == b => Ok(()),
        _ => Err(format!("pid {pid} is {theirs}")),
    }
}

/// The pipe's DACL: this user and SYSTEM, nobody else (the default named-pipe
/// DACL lets Everyone read). `D:P` — protected, nothing inherited.
struct Dacl(PSECURITY_DESCRIPTOR);
impl Dacl {
    fn for_user(sid: &str) -> Option<Dacl> {
        let sddl = HSTRING::from(format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)"));
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(&sddl, SDDL_REVISION_1, &mut sd, None).ok()? };
        Some(Dacl(sd))
    }
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0 .0,
            bInheritHandle: false.into(),
        }
    }
}
impl Drop for Dacl {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0 .0)));
        }
    }
}
unsafe impl Send for Dacl {}

fn last_error() -> u32 {
    unsafe { GetLastError().0 }
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
        let _ = ProcessIdToSessionId(GetCurrentProcessId(), &mut session);
    }
    let mutex_name = HSTRING::from(format!("Local\\vn-instance-{key}"));
    let pipe_name = format!(r"\\.\pipe\vn-instance-{key}-{session}");

    let mut deadline = Instant::now() + patience;
    let mut closing = false;
    loop {
        match take_mutex(&mutex_name) {
            Mutex::Ours => {
                return match serve(pipe_name, sid, deadline) {
                    Ok(()) => Claim::Primary,
                    // We hold the mutex but cannot listen: still the instance
                    // (a second launch will report NoAnswer), and said so.
                    Err(why) => {
                        eprintln!("[launch] the single-instance pipe could not be created: {why}");
                        Claim::Primary
                    }
                };
            }
            Mutex::Denied => {
                return Claim::Unavailable("the instance mutex exists and is not accessible to this user".into())
            }
            Mutex::Failed(why) => return Claim::Unavailable(why),
            Mutex::Taken => {}
        }
        // Somebody holds it. Reach them, or loop and become the instance once the holder has gone.
        match forward(&pipe_name, &sid, fwd, deadline) {
            Sent::Acked => return Claim::Forwarded,
            Sent::Impostor(why) => return Claim::Impostor(why),
            Sent::Closing if !closing || Instant::now() < deadline => {
                if !closing {
                    // Once: an instance that is closing ends itself within
                    // its own deadline, so waiting for it is bounded too.
                    closing = true;
                    deadline = deadline.max(Instant::now() + Duration::from_millis(CLOSING_PATIENCE_MS));
                }
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Sent::Gone if Instant::now() < deadline => {
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

fn take_mutex(name: &HSTRING) -> Mutex {
    match unsafe { CreateMutexW(None, false, name) } {
        Ok(h) => {
            if last_error() == ERROR_ALREADY_EXISTS.0 {
                unsafe {
                    let _ = CloseHandle(h);
                }
                Mutex::Taken
            } else {
                // Held for the life of the process; the kernel drops it when we
                // die, so it can never outlive the instance it names.
                std::mem::forget(Owned(h));
                Mutex::Ours
            }
        }
        Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => Mutex::Denied,
        Err(e) => Mutex::Failed(format!("CreateMutexW: {e}")),
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
    let name = HSTRING::from(pipe_name);
    let pipe = loop {
        let opened = unsafe {
            CreateFileW(
                &name,
                (GENERIC_READ | GENERIC_WRITE).0,
                Default::default(),
                None,
                OPEN_EXISTING,
                // Identification only: the server may ask who we are, never
                // act as us.
                SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };
        match opened {
            Ok(h) => break Owned(h),
            Err(e) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Sent::NoAnswer;
                }
                if e.code() == ERROR_PIPE_BUSY.to_hresult() {
                    unsafe {
                        let _ = WaitNamedPipeW(&name, remaining.as_millis().min(u128::from(u32::MAX)) as u32);
                    }
                    continue;
                }
                if e.code() == ERROR_FILE_NOT_FOUND.to_hresult() {
                    return Sent::Gone;
                }
                return Sent::NoAnswer;
            }
        }
    };
    unsafe {
        let mode = PIPE_READMODE_MESSAGE;
        let _ = SetNamedPipeHandleState(pipe.0, Some(&mode), None, None);
    }
    let mut server_pid = 0u32;
    if unsafe { GetNamedPipeServerProcessId(pipe.0, &mut server_pid) }.is_err() {
        return Sent::Impostor("the pipe server's pid could not be read".into());
    }
    if let Err(why) = verify_server(server_pid, sid) {
        return Sent::Impostor(why);
    }
    // The secondary was just started by the user, so it may hand the
    // foreground on; the primary raises itself on receipt. A quiet launch
    // hands nothing on (and its message says not to raise).
    if fwd.raise {
        unsafe {
            let _ = AllowSetForegroundWindow(server_pid);
        }
    }
    let bytes = fwd.encode();
    if bytes.len() > MAX_MESSAGE {
        eprintln!("[launch] this launch's arguments are over {MAX_MESSAGE} bytes; not forwarded");
        return Sent::NoAnswer;
    }
    let mut written = 0u32;
    if unsafe { WriteFile(pipe.0, Some(&bytes), Some(&mut written), None) }.is_err() || written as usize != bytes.len() {
        return Sent::NoAnswer;
    }
    // The ack, with the deadline: a wedged primary that accepted the
    // connection must not hold this process forever.
    let (tx, rx) = mpsc::channel();
    let raw = pipe.0 .0 as usize;
    std::thread::spawn(move || {
        let mut ack = [0u8; 1];
        let mut n = 0u32;
        let ok = unsafe { ReadFile(HANDLE(raw as *mut c_void), Some(&mut ack), Some(&mut n), None) }.is_ok();
        let _ = tx.send(if ok && n == 1 { ack[0] } else { 0 });
    });
    let remaining = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(250));
    match rx.recv_timeout(remaining) {
        Ok(ACK_DELIVERED) => Sent::Acked,
        Ok(ACK_CLOSING) => Sent::Closing,
        // Leaking the handle on a timeout is deliberate: the reader thread
        // still owns a read on it, and the process exits right after.
        Err(_) => {
            std::mem::forget(pipe);
            Sent::NoAnswer
        }
        Ok(_) => Sent::NoAnswer,
    }
}

const PIPE_BUFFER: u32 = 64 * 1024;

fn create_instance(name: &HSTRING, dacl: &Dacl, first: bool) -> Result<Owned, String> {
    let sa = dacl.attributes();
    let open_mode = if first { PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE } else { PIPE_ACCESS_DUPLEX };
    let h = unsafe {
        CreateNamedPipeW(
            name,
            open_mode,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            Some(&sa),
        )
    };
    if h == INVALID_HANDLE_VALUE || h.is_invalid() {
        let e = last_error();
        return Err(if e == ERROR_ACCESS_DENIED.0 {
            "the pipe name is already taken by another process".to_string()
        } else {
            format!("CreateNamedPipeW failed ({e})")
        });
    }
    Ok(Owned(h))
}

/// Creates the first pipe instance synchronously (so a failure is reported), then hands accepting
/// to the `vn-instance` thread. The mutex can be ours a moment before the previous instance's pipe
/// is gone, so a taken name is retried for up to a second.
fn serve(pipe_name: String, sid: String, deadline: Instant) -> Result<(), String> {
    let dacl = Dacl::for_user(&sid).ok_or("the pipe's security descriptor could not be built")?;
    let name = HSTRING::from(pipe_name);
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
                let connected = match unsafe { ConnectNamedPipe(pipe.0, None) } {
                    Ok(()) => true,
                    Err(e) => e.code() == ERROR_PIPE_CONNECTED.to_hresult(),
                };
                if !connected {
                    continue;
                }
                // The next instance exists BEFORE this client is handled, so a
                // third launch never finds the name missing.
                match create_instance(&name, &dacl, false) {
                    Ok(next) => pending = Some(next),
                    Err(why) => eprintln!("[launch] could not create the next pipe instance: {why}"),
                }
                let sid = sid.clone();
                let _ = std::thread::Builder::new().name("vn-instance-client".into()).spawn(move || handle(pipe, &sid));
            }
        })
        .map_err(|e| format!("spawning the pipe thread: {e}"))?;
    Ok(())
}

fn handle(pipe: Owned, sid: &str) {
    let mut client_pid = 0u32;
    let same_user = unsafe { GetNamedPipeClientProcessId(pipe.0, &mut client_pid) }.is_ok()
        && open_peer(client_pid).and_then(|p| token_user_sid(p.0)).as_deref() == Some(sid);
    if !same_user {
        eprintln!("[launch] refused a single-instance message from pid {client_pid}: not this user");
        unsafe {
            let _ = DisconnectNamedPipe(pipe.0);
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
                let _ = DisconnectNamedPipe(pipe.0);
            }
            return;
        }
    };
    // Closing: the launch is not ours to take. The second process waits for
    // this one to exit and runs as the instance itself.
    let closing = super::is_closing();
    unsafe {
        let mut n = 0u32;
        let _ = WriteFile(pipe.0, Some(&[if closing { ACK_CLOSING } else { ACK_DELIVERED }]), Some(&mut n), None);
        let _ = FlushFileBuffers(pipe.0);
        let _ = DisconnectNamedPipe(pipe.0);
    }
    drop(pipe);
    if closing {
        // stderr, not the log ring: a closing instance drains the ring no
        // more. Once: the second launch asks again every 100 ms.
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
        let r = unsafe { ReadFile(pipe.0, Some(&mut chunk), Some(&mut n), None) };
        out.extend_from_slice(&chunk[..n as usize]);
        if out.len() > MAX_MESSAGE {
            return Err(format!("message over the {MAX_MESSAGE}-byte cap"));
        }
        match r {
            Ok(()) => return Ok(out),
            Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => continue,
            Err(e) => return Err(format!("read failed: {e}")),
        }
    }
}

/// Brings the primary window to the front: restore, show, then take the foreground the secondary
/// allowed (`AllowSetForegroundWindow`).
pub fn raise(hwnd: isize) {
    let hwnd = HWND(hwnd as *mut c_void);
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindowAsync(hwnd, SW_RESTORE);
        } else if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindowAsync(hwnd, SW_SHOW);
        }
        let _ = SetForegroundWindow(hwnd);
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
        assert_ne!(a, instance_key("gl.argon.notes", "notes", "S-1-5-21-2"), "per user");
        assert_ne!(a, instance_key("gl.argon.other", "notes", "S-1-5-21-1"), "per app");
        assert_ne!(a, instance_key("gl.argon.notes", "notes-2", "S-1-5-21-1"), "per storage folder");
        assert_ne!(fnv128(&[b"ab", b"c"]), fnv128(&[b"a", b"bc"]));
    }

    #[test]
    fn our_own_sid_and_image_verify() {
        let sid = own_sid().expect("a SID");
        assert!(sid.starts_with("S-1-"), "{sid}");
        verify_server(unsafe { GetCurrentProcessId() }, &sid).expect("we are ourselves");
        assert!(verify_server(unsafe { GetCurrentProcessId() }, "S-1-0-0").is_err(), "another user");
    }

    #[test]
    fn a_process_with_another_image_is_an_impostor() {
        // Any other running exe of this user: our parent (cargo) will do.
        let parent = std::process::Command::new("cmd").args(["/c", "exit 0"]).spawn();
        if let Ok(mut child) = parent {
            let pid = child.id();
            let sid = own_sid().unwrap();
            let r = verify_server(pid, &sid);
            let _ = child.wait();
            // Either cmd.exe was still there (then it is not us), or it had
            // already exited (then it cannot be opened). Both refuse.
            assert!(r.is_err());
        }
    }

    /// An instance that holds the mutex but never listens (a wedged primary):
    /// the second launch gives up at its deadline instead of starting a copy.
    #[test]
    fn a_holder_that_never_answers_is_no_answer() {
        let id = format!("vn-desktop-wedged-{}", std::process::id());
        let sid = own_sid().unwrap();
        let key = instance_key(&id, "wedged", &sid);
        // Take the mutex the way a primary would, and serve nothing.
        assert!(matches!(take_mutex(&HSTRING::from(format!("Local\\vn-instance-{key}"))), Mutex::Ours));
        let fwd = Forward { argv: vec![], args: vec![], cwd: String::new(), url: None, raise: false };
        let t0 = Instant::now();
        assert!(matches!(claim_within(&id, "wedged", &fwd, Duration::from_millis(400)), Claim::NoAnswer));
        assert!(t0.elapsed() >= Duration::from_millis(350), "it waited for its deadline");
    }

    #[test]
    fn a_second_claim_forwards_to_the_first() {
        let id = format!("vn-desktop-test-{}", std::process::id());
        let fwd = Forward {
            argv: vec!["x.exe".into(), "--y".into()],
            args: vec!["--y".into()],
            cwd: "C:\\".into(),
            url: Some("vntest://a?b=\"c d\"".into()),
            // Never hand this test's foreground on.
            raise: false,
        };
        assert!(matches!(claim(&id, "forward", &fwd), Claim::Primary));
        // The mutex is ours, so a second claim in this process sees it taken
        // (CreateMutexW on an existing name answers ALREADY_EXISTS even to its
        // owner) and forwards over the pipe.
        assert!(matches!(claim(&id, "forward", &fwd), Claim::Forwarded));
        // Delivery lands on the results queue as an unsolicited `launch`.
        let deadline = Instant::now() + Duration::from_secs(5);
        let found = loop {
            let hit = crate::take_emitted().into_iter().find(|p| p.contains("vntest://a?b="));
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
