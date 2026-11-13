//! `VN_OWNER_PID`: a host that dies with the process that launched it.
//!
//! vn-test starts every host detached and kills it from the runner, so a runner that is itself killed
//! (task manager, a crash inside bun) leaves its hosts running, each a full gpui + JSC host holding
//! a window and GPU memory. The host is the only party that outlives the runner, so it does the
//! watching: with `VN_OWNER_PID=<pid>` set, a thread opens a SYNCHRONIZE handle on the owner and
//! blocks on it; when the owner exits, for any reason, the host terminates itself. A handle rather
//! than a pid poll, since the pid can be recycled the moment the owner exits.
//!
//! The termination is `TerminateProcess` on ourselves, not `cx.quit()`: the hosts that leak are
//! disproportionately wedged ones, and a quit that waits for the frame pump never arrives at a stuck
//! pump. A test host has no state worth flushing, and an inspector it opened with F12 inherits the
//! variable (`inspector::spawn` passes the environment through) and goes the same way.

pub fn watch() {
    let Ok(raw) = std::env::var("VN_OWNER_PID") else { return };
    let Ok(pid) = raw.trim().parse::<u32>() else {
        eprintln!("[vn-app] VN_OWNER_PID={raw:?} is not a pid — ignored");
        return;
    };
    imp::watch(pid);
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{ERROR_INVALID_PARAMETER, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, TerminateProcess, WaitForSingleObject, INFINITE,
        PROCESS_SYNCHRONIZE,
    };

    pub fn watch(pid: u32) {
        let handle = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
            Ok(h) => h,
            // "No such process": the owner died between spawning us and this
            // line. That is the case the watch exists for, only earlier.
            Err(e) if e.code() == ERROR_INVALID_PARAMETER.to_hresult() => {
                eprintln!("[vn-app] owner process {pid} is already gone — exiting");
                die();
            }
            // Anything else (access denied across an elevation boundary, say)
            // is a reason to doubt, and a doubt must not kill a running host.
            Err(e) => {
                eprintln!("[vn-app] cannot watch owner process {pid}: {e} — not watching");
                return;
            }
        };
        // A raw HANDLE is a pointer, and pointers are not Send; the handle is
        // only ever waited on from the one thread it is moved to.
        let raw = handle.0 as usize;
        std::thread::Builder::new()
            .name("vn-owner-watch".into())
            .spawn(move || {
                let handle = HANDLE(raw as *mut core::ffi::c_void);
                if unsafe { WaitForSingleObject(handle, INFINITE) } == WAIT_OBJECT_0 {
                    eprintln!("[vn-app] owner process {pid} exited — exiting with it");
                    die();
                }
            })
            .ok();
    }

    fn die() -> ! {
        unsafe {
            let _ = TerminateProcess(GetCurrentProcess(), 0);
        }
        std::process::exit(0)
    }
}

#[cfg(unix)]
mod imp {
    /// Unix has no handle to wait on without a new dependency, but it has the
    /// one fact that matters for a launcher: when a parent dies its children
    /// are re-parented, so `parent_id()` stops answering the owner's pid. That
    /// only holds for a DIRECT child, so anything else is told so and left alone.
    pub fn watch(pid: u32) {
        if std::os::unix::process::parent_id() != pid {
            eprintln!(
                "[vn-app] VN_OWNER_PID={pid} is not this process's parent — on unix only a \
                 direct parent can be watched; not watching"
            );
            return;
        }
        std::thread::Builder::new()
            .name("vn-owner-watch".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if std::os::unix::process::parent_id() != pid {
                    eprintln!("[vn-app] owner process {pid} exited — exiting with it");
                    std::process::exit(0);
                }
            })
            .ok();
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    pub fn watch(pid: u32) {
        eprintln!("[vn-app] VN_OWNER_PID={pid}: not supported on this platform — not watching");
    }
}
