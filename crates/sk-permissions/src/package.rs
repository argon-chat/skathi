//! The MSIX package identity this process runs with, if any. It is fixed for the life of a process.

/// The package family name (`<Name>_<publisher hash>`), or `None` for an unpackaged process.
pub fn family_name() -> Option<&'static str> {
    #[cfg(target_os = "windows")]
    {
        static NAME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        NAME.get_or_init(|| {
            win::read(|len, buf| unsafe { sk_windows::raw::GetCurrentPackageFamilyName(len, buf) })
        })
        .as_deref()
    }
    #[cfg(not(target_os = "windows"))]
    None
}

pub fn has_identity() -> bool {
    family_name().is_some()
}

/// The AUMID Windows gave this packaged process (`<family>!<application id>`).
pub fn app_user_model_id() -> Option<&'static str> {
    #[cfg(target_os = "windows")]
    {
        static ID: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        ID.get_or_init(|| {
            win::read(|len, buf| unsafe {
                sk_windows::raw::GetCurrentApplicationUserModelId(len, buf)
            })
        })
        .as_deref()
    }
    #[cfg(not(target_os = "windows"))]
    None
}

#[cfg(target_os = "windows")]
mod win {
    use sk_windows::raw::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};

    /// The two-call string pattern of the `GetCurrentPackage*` family; an unpackaged process gets
    /// `APPMODEL_ERROR_NO_PACKAGE` from the first call.
    pub fn read(get: impl Fn(*mut u32, *mut u16) -> u32) -> Option<String> {
        let mut len = 0u32;
        if get(&mut len, std::ptr::null_mut()) != ERROR_INSUFFICIENT_BUFFER {
            return None;
        }
        let mut buf = vec![0u16; len as usize];
        if get(&mut len, buf.as_mut_ptr()) != ERROR_SUCCESS {
            return None;
        }
        let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_test_process_has_no_package_identity() {
        assert_eq!(super::family_name(), None);
        assert_eq!(super::app_user_model_id(), None);
    }
}
