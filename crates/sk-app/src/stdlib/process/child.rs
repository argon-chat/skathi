//! A started program: its OS handle, how it ended, and ending it. Never through a shell.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

#[cfg(unix)]
use super::unix as imp;
#[cfg(windows)]
use super::win as imp;

/// Children running at once; one more is refused.
pub(super) const LIVE_LIMIT: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Stdio {
    Pipe,
    Null,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Signal {
    Term,
    Kill,
}

impl Signal {
    pub fn name(self) -> &'static str {
        match self {
            Signal::Term => "SIGTERM",
            Signal::Kill => "SIGKILL",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Exit {
    Code(i64),
    Signal(String),
}

pub(super) struct Request<'a> {
    /// Canonical, and matched against the allow-list.
    pub program: &'a Path,
    pub args: &'a [String],
    /// The child's whole environment.
    pub env: Vec<(OsString, OsString)>,
    pub cwd: Option<&'a Path>,
    pub stdin: Stdio,
    pub stdout: Stdio,
    pub stderr: Stdio,
}

pub(super) struct Spawned {
    pub child: Arc<Child>,
    pub stdin: Option<File>,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
}

#[derive(Default)]
pub(super) struct State {
    pub exited: bool,
    pub killed: Option<Signal>,
}

pub(super) struct Child {
    pub pid: u32,
    pub(super) os: imp::Handle,
    /// Held while signalling, and while the exit is recorded: on Unix the pid is not reaped (and
    /// cannot be reused) until `exited` is set under this lock.
    pub(super) state: Mutex<State>,
}

static LIVE: AtomicUsize = AtomicUsize::new(0);
static ALL: Mutex<Vec<Weak<Child>>> = Mutex::new(Vec::new());

pub(super) fn live() -> usize {
    LIVE.load(Ordering::Acquire)
}

pub(super) fn spawn(request: Request<'_>) -> io::Result<Spawned> {
    let spawned = imp::spawn(&request)?;
    LIVE.fetch_add(1, Ordering::AcqRel);
    let mut all = ALL.lock().unwrap_or_else(|e| e.into_inner());
    all.retain(|c| c.strong_count() > 0);
    all.push(Arc::downgrade(&spawned.child));
    Ok(spawned)
}

impl Child {
    /// Ends the child and what it started, unless it has exited. A no-op once it has.
    pub fn kill(&self, signal: Signal) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.exited || imp::has_exited(&self.os) {
            return;
        }
        state.killed = Some(signal);
        imp::kill(&self.os, self.pid, signal);
    }

    /// Ends whatever of the child's tree is left: the child if it runs, and on Windows also what
    /// it started and left behind.
    pub fn end(&self) {
        self.kill(Signal::Kill);
        imp::end_tree(&self.os);
    }

    /// Blocks until the child exits, and reaps it. Called once, by the thread that waits for it.
    pub fn wait(&self) -> Exit {
        let exit = imp::wait(self);
        LIVE.fetch_sub(1, Ordering::AcqRel);
        exit
    }

    pub fn exited(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).exited
    }
}

/// The quit hook: every child gets SIGTERM, then SIGKILL after `grace`, and on Windows its job ends.
pub(super) fn end_all(grace: Duration) {
    let children: Vec<Arc<Child>> = ALL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .filter_map(|c| c.upgrade())
        .collect();
    for child in &children {
        child.kill(Signal::Term);
    }
    let deadline = Instant::now() + grace;
    while children.iter().any(|c| !c.exited()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    for child in &children {
        child.end();
    }
}
