use std::ffi::CString;
use std::fmt;
use std::path::Path;
use std::sync::Mutex;

/// Loading and symbol lookup are not safe to run from several threads at once on every platform.
static LOADER: Mutex<()> = Mutex::new(());

/// A loaded library. It is never unloaded: a function pointer or a callback may outlive any
/// moment at which unloading would look safe.
#[derive(Debug)]
pub struct Library {
    handle: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadError {
    pub library: String,
    pub symbol: Option<String>,
    /// The system's own words.
    pub message: String,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.symbol {
            Some(symbol) => write!(
                f,
                "{} has no symbol {symbol}: {}",
                self.library, self.message
            ),
            None => write!(
                f,
                "could not load {} or one of the libraries it needs: {}",
                self.library, self.message
            ),
        }
    }
}

impl std::error::Error for LoadError {}

pub(crate) fn last_os_error() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

impl Library {
    /// `path` must be absolute: a bare name would be searched for in places an app does not control.
    pub fn open(path: &Path) -> Result<Library, LoadError> {
        let failed = |message: String| LoadError {
            library: path.display().to_string(),
            symbol: None,
            message,
        };
        if !path.is_absolute() {
            return Err(failed("the path is not absolute".into()));
        }
        let _guard = LOADER.lock().unwrap_or_else(|e| e.into_inner());
        let handle = unsafe { sys::open(path) }.map_err(failed)?;
        Ok(Library { handle })
    }

    /// A library the system's loader finds in its own folders: `name` has no folder in it.
    pub fn open_system(name: &str) -> Result<Library, LoadError> {
        let failed = |message: String| LoadError {
            library: name.to_string(),
            symbol: None,
            message,
        };
        if name.is_empty() || name.contains(['/', '\\', ':']) {
            return Err(failed("the name has a folder in it".into()));
        }
        let _guard = LOADER.lock().unwrap_or_else(|e| e.into_inner());
        let handle = unsafe { sys::open_system(name) }.map_err(failed)?;
        Ok(Library { handle })
    }

    pub fn symbol(&self, name: &str) -> Option<*const ()> {
        let name = CString::new(name).ok()?;
        let _guard = LOADER.lock().unwrap_or_else(|e| e.into_inner());
        let address = unsafe { sys::symbol(self.handle, &name) };
        (!address.is_null()).then_some(address)
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::CStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
        LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
    };

    pub unsafe fn open_system(name: &str) -> Result<usize, String> {
        unsafe {
            let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let handle = LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            );
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(handle as usize)
        }
    }

    pub unsafe fn open(path: &Path) -> Result<usize, String> {
        unsafe {
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
            // The library's own folder first, so the DLLs shipped beside it are found.
            let flags = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS;
            let handle = LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), flags);
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(handle as usize)
        }
    }

    pub unsafe fn symbol(handle: usize, name: &CStr) -> *const () {
        unsafe {
            match GetProcAddress(handle as _, name.as_ptr().cast()) {
                Some(function) => function as *const (),
                None => std::ptr::null(),
            }
        }
    }
}

#[cfg(unix)]
mod sys {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub unsafe fn open_system(name: &str) -> Result<usize, String> {
        unsafe { open(Path::new(name)) }
    }

    pub unsafe fn open(path: &Path) -> Result<usize, String> {
        unsafe {
            let c_path = CString::new(path.as_os_str().as_bytes())
                .map_err(|_| "the path has a NUL in it".to_string())?;
            let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if handle.is_null() {
                let message = libc::dlerror();
                return Err(if message.is_null() {
                    "dlopen failed".into()
                } else {
                    CStr::from_ptr(message).to_string_lossy().into_owned()
                });
            }
            Ok(handle as usize)
        }
    }

    pub unsafe fn symbol(handle: usize, name: &CStr) -> *const () {
        unsafe { libc::dlsym(handle as *mut _, name.as_ptr()) as *const () }
    }
}
