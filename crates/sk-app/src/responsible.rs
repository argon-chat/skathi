//! A process started from a terminal is the terminal's for TCC: macOS asks on the terminal's behalf
//! and reads the terminal's Info.plist. A host inside an `.app` (a built app run from a shell, or the
//! bundle `sk dev` runs) re-executes itself in place as its own responsible process, so the prompt,
//! the usage description and the remembered decision are the app's.

use std::ffi::{c_char, c_int, c_void, CString};
use std::os::unix::ffi::OsStrExt;

const MARK: &str = "SK_RESPONSIBLE";
const POSIX_SPAWN_SETEXEC: libc::c_short = 0x0040;

pub(crate) fn claim() {
    if std::env::var_os(MARK).is_some() {
        return;
    }
    // Launched by launchd (Finder, Dock, `open`): already its own.
    if unsafe { libc::getppid() } == 1 {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if !in_app_bundle(&exe) {
        return;
    }
    // SAFETY: a lookup by a NUL-terminated name in the already loaded images.
    let sym = unsafe {
        libc::dlsym(
            libc::RTLD_DEFAULT,
            c"responsibility_spawnattrs_setdisclaim".as_ptr(),
        )
    };
    if sym.is_null() {
        return;
    }
    // SAFETY: libSystem's `int responsibility_spawnattrs_setdisclaim(posix_spawnattr_t *, int)`.
    let disclaim: unsafe extern "C" fn(*mut libc::posix_spawnattr_t, c_int) -> c_int =
        unsafe { std::mem::transmute::<*mut c_void, _>(sym) };

    let cstr = |b: &[u8]| CString::new(b).ok();
    let Some(path) = cstr(exe.as_os_str().as_bytes()) else {
        return;
    };
    let Some(args) = std::env::args_os()
        .map(|a| cstr(a.as_bytes()))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    let Some(mut env) = std::env::vars_os()
        .filter(|(k, _)| k != MARK)
        .map(|(k, v)| cstr(&[k.as_bytes(), b"=", v.as_bytes()].concat()))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    env.extend(cstr(format!("{MARK}=1").as_bytes()));
    let argv: Vec<*mut c_char> = args
        .iter()
        .map(|a| a.as_ptr() as *mut c_char)
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();
    let envp: Vec<*mut c_char> = env
        .iter()
        .map(|e| e.as_ptr() as *mut c_char)
        .chain(std::iter::once(std::ptr::null_mut()))
        .collect();
    // SAFETY: every pointer outlives the call; with POSIX_SPAWN_SETEXEC it returns only on failure,
    // and the process then carries on as it was.
    unsafe {
        let mut attr: libc::posix_spawnattr_t = std::ptr::null_mut();
        if libc::posix_spawnattr_init(&mut attr) != 0 {
            return;
        }
        if libc::posix_spawnattr_setflags(&mut attr, POSIX_SPAWN_SETEXEC) == 0
            && disclaim(&mut attr, 1) == 0
        {
            let mut pid: libc::pid_t = 0;
            let err = libc::posix_spawn(
                &mut pid,
                path.as_ptr(),
                std::ptr::null(),
                &attr,
                argv.as_ptr(),
                envp.as_ptr(),
            );
            eprintln!("[host] could not re-execute as the app's own process: errno {err}");
        }
        libc::posix_spawnattr_destroy(&mut attr);
    }
}

fn in_app_bundle(exe: &std::path::Path) -> bool {
    let Some(macos) = exe.parent() else {
        return false;
    };
    let Some(contents) = macos.parent() else {
        return false;
    };
    macos.ends_with("MacOS")
        && contents.ends_with("Contents")
        && contents
            .parent()
            .and_then(std::path::Path::extension)
            .is_some_and(|e| e.eq_ignore_ascii_case("app"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_an_executable_in_contents_macos_of_an_app_is_bundled() {
        use std::path::Path;
        assert!(super::in_app_bundle(Path::new(
            "/x/Demo.app/Contents/MacOS/sk-app"
        )));
        assert!(!super::in_app_bundle(Path::new("/x/target/debug/sk-app")));
        assert!(!super::in_app_bundle(Path::new(
            "/x/Demo/Contents/MacOS/sk-app"
        )));
    }
}
