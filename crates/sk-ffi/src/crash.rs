//! Names the foreign function a fatal fault happened in. The fault itself is not survived.

use std::cell::Cell;
use std::ffi::CStr;

thread_local! {
    static CURRENT: Cell<Option<&'static CStr>> = const { Cell::new(None) };
}

/// Runs `call` with `label` (`library!symbol`) as what a crash on this thread is blamed on.
pub fn in_call<R>(label: &'static CStr, call: impl FnOnce() -> R) -> R {
    install();
    let outer = CURRENT.replace(Some(label));
    let result = call();
    CURRENT.set(outer);
    result
}

#[cfg(windows)]
fn install() {
    use sk_windows::raw::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
    };
    use sk_windows::raw::{
        EXCEPTION_ACCESS_VIOLATION, EXCEPTION_ILLEGAL_INSTRUCTION, EXCEPTION_INT_DIVIDE_BY_ZERO,
        EXCEPTION_IN_PAGE_ERROR, EXCEPTION_PRIV_INSTRUCTION, EXCEPTION_STACK_OVERFLOW,
    };
    use std::sync::Once;

    unsafe extern "system" fn report(info: *mut EXCEPTION_POINTERS) -> i32 {
        unsafe {
            let code = (*(*info).ExceptionRecord).ExceptionCode;
            let fatal = [
                EXCEPTION_ACCESS_VIOLATION,
                EXCEPTION_ILLEGAL_INSTRUCTION,
                EXCEPTION_INT_DIVIDE_BY_ZERO,
                EXCEPTION_IN_PAGE_ERROR,
                EXCEPTION_PRIV_INSTRUCTION,
                EXCEPTION_STACK_OVERFLOW,
            ]
            .contains(&code);
            if fatal {
                if let Ok(Some(label)) = CURRENT.try_with(Cell::get) {
                    write_line(label);
                }
            }
            EXCEPTION_CONTINUE_SEARCH
        }
    }

    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(report));
    });
}

#[cfg(unix)]
mod posix {
    use super::{write_line, CURRENT};
    use std::cell::Cell;
    use std::mem::MaybeUninit;
    use std::sync::Once;

    const SIGNALS: [libc::c_int; 4] = [libc::SIGSEGV, libc::SIGBUS, libc::SIGILL, libc::SIGFPE];

    /// What was installed before, by the engine or the embedder: the fault is theirs after the line.
    static mut PREVIOUS: [MaybeUninit<libc::sigaction>; 4] = [MaybeUninit::uninit(); 4];

    unsafe extern "C" fn report(
        signal: libc::c_int,
        info: *mut libc::siginfo_t,
        context: *mut libc::c_void,
    ) {
        unsafe {
            if let Ok(Some(label)) = CURRENT.try_with(Cell::get) {
                // A handler that returns into the fault comes here again.
                let _ = CURRENT.try_with(|current| current.set(None));
                write_line(label);
            }
            let Some(index) = SIGNALS.iter().position(|s| *s == signal) else {
                return;
            };
            // SAFETY: written once, before the handler was installed.
            let previous = (*std::ptr::addr_of!(PREVIOUS))[index].assume_init();
            let handler = previous.sa_sigaction;
            if handler == libc::SIG_DFL || handler == libc::SIG_IGN {
                // Returning runs the faulting instruction again, now with the default action.
                libc::sigaction(signal, &previous, std::ptr::null_mut());
            } else if previous.sa_flags & libc::SA_SIGINFO != 0 {
                let chained: unsafe extern "C" fn(
                    libc::c_int,
                    *mut libc::siginfo_t,
                    *mut libc::c_void,
                ) = std::mem::transmute(handler);
                chained(signal, info, context);
            } else {
                let chained: unsafe extern "C" fn(libc::c_int) = std::mem::transmute(handler);
                chained(signal);
            }
        }
    }

    pub fn install() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| unsafe {
            for (index, signal) in SIGNALS.iter().enumerate() {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = report as *const () as usize;
                action.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK | libc::SA_NODEFER;
                libc::sigemptyset(&mut action.sa_mask);
                let slot = (*std::ptr::addr_of_mut!(PREVIOUS))[index].as_mut_ptr();
                libc::sigaction(*signal, &action, slot);
            }
        });
    }
}

#[cfg(unix)]
use posix::install;

#[cfg(not(any(windows, unix)))]
fn install() {}

/// Without allocating or locking: the heap may be what is broken, and on POSIX this is a signal handler.
fn write_line(label: &CStr) {
    let mut line = [0u8; 256];
    let mut written = 0;
    for part in [&b"[crash] ffi: "[..], label.to_bytes(), b"\n"] {
        let take = part.len().min(line.len() - written);
        line[written..written + take].copy_from_slice(&part[..take]);
        written += take;
    }
    #[cfg(unix)]
    unsafe {
        libc::write(2, line.as_ptr().cast(), written);
    }
    #[cfg(not(unix))]
    {
        use std::io::Write;
        let _ = std::io::stderr().write_all(&line[..written]);
    }
}
