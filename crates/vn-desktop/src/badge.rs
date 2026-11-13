//! The app badge: the taskbar button's overlay icon on Windows, the Dock tile's label on macOS. Counts
//! show as Chromium shows them: up to 99, then "99+".

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "macos")]
pub use mac::set;
#[cfg(target_os = "windows")]
pub use win::set;

/// `None` clears; `""` is the flag.
pub fn label(contents: i64) -> Option<String> {
    match contents {
        0 => None,
        c if c < 0 => Some(String::new()),
        1..=99 => Some(contents.to_string()),
        _ => Some("99+".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_cap_at_99_and_zero_clears() {
        assert_eq!(label(0), None);
        assert_eq!(label(-1).as_deref(), Some(""));
        assert_eq!(label(7).as_deref(), Some("7"));
        assert_eq!(label(99).as_deref(), Some("99"));
        assert_eq!(label(100).as_deref(), Some("99+"));
    }
}
