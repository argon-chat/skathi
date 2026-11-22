//! fork + execve through `std::process` with the program's absolute path (no PATH search), the
//! child leading a process group of its own, and every descriptor above stderr closed on exec.

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio as StdStdio};
use std::sync::{Arc, Mutex};

use super::child::{Child, Exit, Request, Signal, Spawned, State, Stdio};

pub(super) struct Handle;

/// Refuses a file the kernel would not run itself: `execvp` hands one without a known header to
/// `/bin/sh`.
fn check_header(program: &std::path::Path) -> io::Result<()> {
    let mut magic = [0u8; 4];
    let read = File::open(program)?.read(&mut magic)?;
    let known = [
        *b"\x7fELF",
        [0xfe, 0xed, 0xfa, 0xce],
        [0xce, 0xfa, 0xed, 0xfe],
        [0xfe, 0xed, 0xfa, 0xcf],
        [0xcf, 0xfa, 0xed, 0xfe],
        [0xca, 0xfe, 0xba, 0xbe],
        [0xbe, 0xba, 0xfe, 0xca],
    ];
    if (read == 4 && known.contains(&magic)) || (read >= 2 && &magic[..2] == b"#!") {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "the program is neither an executable nor a script that starts with #!",
    ))
}

fn stdio(mode: Stdio) -> StdStdio {
    match mode {
        Stdio::Pipe => StdStdio::piped(),
        Stdio::Null => StdStdio::null(),
    }
}

fn open_max() -> i32 {
    // SAFETY: a plain query.
    let n = unsafe { libc::sysconf(libc::_SC_OPEN_MAX) };
    n.clamp(256, 65_536) as i32
}

/// Async-signal-safe: runs between fork and exec.
fn close_on_exec_above_stderr(limit: i32) {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: close_range only flags descriptors; a kernel without it answers ENOSYS.
        let done = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                3u32,
                u32::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            )
        };
        if done == 0 {
            return;
        }
    }
    for fd in 3..limit {
        // SAFETY: fcntl on a descriptor that may not be open answers EBADF.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

pub(super) fn spawn(request: &Request<'_>) -> io::Result<Spawned> {
    check_header(request.program)?;
    let mut command = Command::new(request.program);
    command
        .args(request.args)
        .env_clear()
        .envs(request.env.iter().map(|(k, v)| (k, v)))
        .stdin(stdio(request.stdin))
        .stdout(stdio(request.stdout))
        .stderr(stdio(request.stderr))
        .process_group(0);
    if let Some(cwd) = request.cwd {
        command.current_dir(cwd);
    }
    let limit = open_max();
    #[cfg(target_os = "linux")]
    // SAFETY: a plain query.
    let parent = unsafe { libc::getpid() };
    // SAFETY: the closure calls only async-signal-safe functions and allocates nothing.
    unsafe {
        command.pre_exec(move || {
            #[cfg(target_os = "linux")]
            {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                if libc::getppid() != parent {
                    return Err(io::Error::from_raw_os_error(libc::ESRCH));
                }
            }
            close_on_exec_above_stderr(limit);
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let file = |fd: Option<OwnedFd>| fd.map(File::from);
    Ok(Spawned {
        stdin: file(child.stdin.take().map(OwnedFd::from)),
        stdout: file(child.stdout.take().map(OwnedFd::from)),
        stderr: file(child.stderr.take().map(OwnedFd::from)),
        child: Arc::new(Child {
            pid: child.id(),
            os: Handle,
            state: Mutex::new(State::default()),
        }),
    })
}

pub(super) fn has_exited(_os: &Handle) -> bool {
    false
}

pub(super) fn kill(_os: &Handle, pid: u32, signal: Signal) {
    let number = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // SAFETY: the group's leader is not reaped while the caller holds the state lock, so the id
    // still names it.
    unsafe { libc::killpg(pid as libc::pid_t, number) };
}

/// A grandchild that outlives the child is not followed: once the child is reaped its group id
/// may name an unrelated group.
pub(super) fn end_tree(_os: &Handle) {}

pub(super) fn wait(child: &Child) -> Exit {
    let pid = child.pid as libc::pid_t;
    loop {
        // SAFETY: an out-structure; WNOWAIT leaves the child unreaped.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let r = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if r == 0 || io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            break;
        }
    }
    let mut state = child.state.lock().unwrap_or_else(|e| e.into_inner());
    let mut status = 0;
    loop {
        // SAFETY: reaps the child this module started.
        let r = unsafe { libc::waitpid(pid, &mut status, 0) };
        if r >= 0 || io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            break;
        }
    }
    state.exited = true;
    if libc::WIFSIGNALED(status) {
        Exit::Signal(signal_name(libc::WTERMSIG(status)))
    } else {
        Exit::Code(libc::WEXITSTATUS(status) as i64)
    }
}

pub(super) fn signal_name(n: i32) -> String {
    let name = match n {
        libc::SIGHUP => "SIGHUP",
        libc::SIGINT => "SIGINT",
        libc::SIGQUIT => "SIGQUIT",
        libc::SIGILL => "SIGILL",
        libc::SIGTRAP => "SIGTRAP",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        libc::SIGFPE => "SIGFPE",
        libc::SIGKILL => "SIGKILL",
        libc::SIGUSR1 => "SIGUSR1",
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGUSR2 => "SIGUSR2",
        libc::SIGPIPE => "SIGPIPE",
        libc::SIGALRM => "SIGALRM",
        libc::SIGTERM => "SIGTERM",
        _ => return format!("SIG{n}"),
    };
    name.into()
}
