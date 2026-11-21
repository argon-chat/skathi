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
    use std::sync::Once;
    use windows_sys::Win32::Foundation::{
        EXCEPTION_ACCESS_VIOLATION, EXCEPTION_ILLEGAL_INSTRUCTION, EXCEPTION_INT_DIVIDE_BY_ZERO,
        EXCEPTION_IN_PAGE_ERROR, EXCEPTION_PRIV_INSTRUCTION, EXCEPTION_STACK_OVERFLOW,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
    };

    unsafe extern "system" fn report(info: *mut EXCEPTION_POINTERS) -> i32 {
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

    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(report));
    });
}

/// No handler on POSIX yet: the engine owns the fault signals there.
#[cfg(not(windows))]
fn install() {}

/// Without allocating: the heap may be what is broken.
#[cfg(windows)]
fn write_line(label: &CStr) {
    use std::io::Write;
    let mut line = [0u8; 256];
    let mut cursor = &mut line[..];
    let _ = cursor.write_all(b"[crash] ffi: ");
    let _ = cursor.write_all(label.to_bytes());
    let _ = cursor.write_all(b"\n");
    let left = cursor.len();
    let written = line.len() - left;
    let _ = std::io::stderr().write_all(&line[..written]);
}
