//! The one-instance rule on Linux: `instance_win`'s protocol over a Unix socket.
//!
//! `$XDG_RUNTIME_DIR/vn-instance-<key>.lock` is flock'd by the instance for its lifetime and `.sock` is
//! where it listens. The `flock` decides, not the socket: the kernel drops it however the holder ends, so a
//! stale socket from a crash is replaced. The runtime dir is per user (else a 0700 `/tmp/vn-instance-<uid>`)
//! and the server checks the peer's uid (`SO_PEERCRED`) before reading.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{Forward, CLOSING_PATIENCE_MS, FORWARD_TIMEOUT_MS, MAX_MESSAGE};

#[derive(Debug)]
pub enum Claim {
    Primary,
    Unavailable(String),
    Forwarded,
    NoAnswer,
    StillClosing,
    Impostor(String),
}

const ACK_DELIVERED: u8 = 1;
const ACK_CLOSING: u8 = 2;

fn instance_key(id: &str, storage: &str) -> String {
    let mut h: u128 = 0x6c62272e07bb014262b821756295c58d;
    for (i, part) in [id.as_bytes(), storage.as_bytes()].into_iter().enumerate() {
        if i > 0 {
            h ^= 0;
            h = h.wrapping_mul(0x0000000001000000000000000000013B);
        }
        for &b in part {
            h ^= u128::from(b);
            h = h.wrapping_mul(0x0000000001000000000000000000013B);
        }
    }
    format!("{h:032x}")
}

fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

fn runtime_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute() && p.is_dir()) {
        return Ok(dir);
    }
    let dir = std::env::temp_dir().join(format!("vn-instance-{}", uid()));
    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("creating {}: {e}", dir.display())),
    }
    // Someone else's directory under a predictable name is not ours to use.
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !meta.is_dir() || meta.uid() != uid() || meta.mode() & 0o077 != 0 {
        return Err(format!("{} is not a private directory of this user", dir.display()));
    }
    Ok(dir)
}

pub fn claim(id: &str, storage: &str, fwd: &Forward) -> Claim {
    claim_within(id, storage, fwd, Duration::from_millis(FORWARD_TIMEOUT_MS))
}

fn claim_within(id: &str, storage: &str, fwd: &Forward, patience: Duration) -> Claim {
    let dir = match runtime_dir() {
        Ok(dir) => dir,
        Err(why) => return Claim::Unavailable(why),
    };
    let key = instance_key(id, storage);
    let lock_path = dir.join(format!("vn-instance-{key}.lock"));
    let sock_path = dir.join(format!("vn-instance-{key}.sock"));

    let mut deadline = Instant::now() + patience;
    let mut closing = false;
    loop {
        match take_lock(&lock_path) {
            Lock::Ours(file) => {
                std::mem::forget(file);
                if let Err(why) = serve(&sock_path) {
                    eprintln!("[launch] the single-instance socket could not be created: {why}");
                }
                return Claim::Primary;
            }
            Lock::Failed(why) => return Claim::Unavailable(why),
            Lock::Taken => {}
        }
        match forward(&sock_path, fwd, deadline) {
            Sent::Acked => return Claim::Forwarded,
            Sent::Impostor(why) => return Claim::Impostor(why),
            Sent::Closing if !closing || Instant::now() < deadline => {
                if !closing {
                    closing = true;
                    deadline = deadline.max(Instant::now() + Duration::from_millis(CLOSING_PATIENCE_MS));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Sent::Gone if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Sent::Closing => return Claim::StillClosing,
            Sent::Gone | Sent::NoAnswer if closing => return Claim::StillClosing,
            Sent::Gone | Sent::NoAnswer => return Claim::NoAnswer,
        }
    }
}

enum Lock {
    Ours(File),
    Taken,
    Failed(String),
}

fn take_lock(path: &PathBuf) -> Lock {
    let file = match OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(path) {
        Ok(f) => f,
        Err(e) => return Lock::Failed(format!("opening {}: {e}", path.display())),
    };
    // SAFETY: a valid fd we own.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Lock::Ours(file);
    }
    match std::io::Error::last_os_error() {
        e if e.kind() == ErrorKind::WouldBlock => Lock::Taken,
        e => Lock::Failed(format!("locking {}: {e}", path.display())),
    }
}

enum Sent {
    Acked,
    Closing,
    Gone,
    NoAnswer,
    Impostor(String),
}

fn forward(sock: &PathBuf, fwd: &Forward, deadline: Instant) -> Sent {
    let mut stream = match UnixStream::connect(sock) {
        Ok(s) => s,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => return Sent::Gone,
        Err(_) => return Sent::NoAnswer,
    };
    match peer_uid(&stream) {
        Some(peer) if peer == uid() => {}
        Some(peer) => return Sent::Impostor(format!("the socket's server runs as uid {peer}")),
        None => return Sent::Impostor("the socket's server could not be identified".into()),
    }
    let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(100));
    let _ = stream.set_write_timeout(Some(left));
    let _ = stream.set_read_timeout(Some(left));
    if stream.write_all(&fwd.encode()).is_err() || stream.shutdown(std::net::Shutdown::Write).is_err() {
        return Sent::NoAnswer;
    }
    let mut ack = [0u8; 1];
    match stream.read_exact(&mut ack) {
        Ok(()) if ack[0] == ACK_DELIVERED => Sent::Acked,
        Ok(()) if ack[0] == ACK_CLOSING => Sent::Closing,
        Ok(()) => Sent::Impostor(format!("unexpected answer {}", ack[0])),
        Err(_) => Sent::NoAnswer,
    }
}

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: a connected socket and a correctly sized out struct.
    let r = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    (r == 0).then_some(cred.uid)
}

fn serve(sock: &PathBuf) -> Result<(), String> {
    // Only the lock holder gets here, so a file at this path is a dead one's.
    let _ = fs::remove_file(sock);
    let listener = UnixListener::bind(sock).map_err(|e| format!("binding {}: {e}", sock.display()))?;
    std::thread::Builder::new()
        .name("vn-instance".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let _ = std::thread::Builder::new().name("vn-instance-client".into()).spawn(move || handle(stream));
                    }
                    Err(e) => eprintln!("[launch] the single-instance socket failed to accept: {e}"),
                }
            }
        })
        .map_err(|e| format!("spawning the socket thread: {e}"))?;
    Ok(())
}

fn handle(mut stream: UnixStream) {
    if peer_uid(&stream) != Some(uid()) {
        eprintln!("[launch] refused a single-instance message: not this user");
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_millis(FORWARD_TIMEOUT_MS)));
    let mut bytes = Vec::new();
    let read = (&mut stream).take(MAX_MESSAGE as u64 + 1).read_to_end(&mut bytes);
    let msg = match read {
        Ok(_) => Forward::decode(&bytes),
        Err(e) => Err(format!("read failed: {e}")),
    };
    let msg = match msg {
        Ok(m) => m,
        Err(why) => {
            eprintln!("[launch] refused a single-instance message: {why}");
            return;
        }
    };
    let closing = super::is_closing();
    let _ = stream.write_all(&[if closing { ACK_CLOSING } else { ACK_DELIVERED }]);
    drop(stream);
    if closing {
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("[launch] a second launch arrived while this instance is closing; it starts the app once this one has ended");
        }
        return;
    }
    super::deliver(msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fwd(url: &str) -> Forward {
        Forward { argv: vec!["app".into()], args: vec![], cwd: "/".into(), url: Some(url.into()), raise: false }
    }

    #[test]
    fn the_key_separates_its_parts() {
        assert_eq!(instance_key("a", "b"), instance_key("a", "b"));
        assert_ne!(instance_key("ab", ""), instance_key("a", "b"));
    }

    #[test]
    fn a_second_claim_forwards_to_the_first() {
        let id = format!("sk-test-{}", std::process::id());
        assert!(matches!(claim_within(&id, "s", &fwd("x://1"), Duration::from_secs(2)), Claim::Primary));
        assert!(matches!(claim_within(&id, "s", &fwd("x://2"), Duration::from_secs(2)), Claim::Forwarded));
    }
}
