//! `navigator.clipboard` backend: gpui's `App::read_from_clipboard` / `write_to_clipboard` behind redjs's
//! `ClipboardHost`. Text goes through gpui because `OpenClipboard` is exclusive and the native <input> copies
//! through gpui. Items (`win.rs`) open the clipboard themselves (gpui's `ClipboardItem` has no HTML entry);
//! both paths open and close around one call on the same thread, so they never overlap. gpui's clipboard
//! needs an `&App`, available only inside a published frame scope; a call from outside rejects.

use gpui::ClipboardItem;
use vn_runtime::redjs::webapi::clipboard::{self as api, ClipboardHost};
use vn_runtime::with_app;

pub struct GpuiClipboard;

const NO_FRAME: &str = "the clipboard is only reachable from inside a frame (this call ran \
                        outside the gpui publication — see crates/vn-runtime/src/bridge.rs)";

impl ClipboardHost for GpuiClipboard {
    fn platform(&self) -> &'static str {
        if cfg!(target_os = "windows") {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "this platform"
        }
    }

    fn read_text(&self) -> Result<String, String> {
        with_app(|cx| {
            cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default()
        })
        .ok_or_else(|| NO_FRAME.to_string())
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        with_app(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text.to_string())))
            .ok_or_else(|| NO_FRAME.to_string())
    }

    fn item_types(&self) -> &'static [&'static str] {
        #[cfg(windows)]
        return win::TYPES;
        #[cfg(not(windows))]
        return &[];
    }

    fn write_items(&self, items: &[(String, Vec<u8>)]) -> Result<(), String> {
        #[cfg(windows)]
        return win::write(items);
        #[cfg(not(windows))]
        {
            let _ = items;
            Err(format!("{} has no item clipboard in this build", self.platform()))
        }
    }

    fn read_items(&self) -> Result<Vec<(String, Vec<u8>)>, String> {
        #[cfg(windows)]
        return win::read();
        #[cfg(not(windows))]
        {
            let text = self.read_text()?;
            Ok(if text.is_empty() { Vec::new() } else { vec![("text/plain".to_string(), text.into_bytes())] })
        }
    }
}

#[cfg(windows)]
mod win;

/// Installs the backend into redjs. Registers no host functions and needs no pump.
pub fn install() {
    static H: GpuiClipboard = GpuiClipboard;
    api::set_host(&H);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_names_the_platform_it_is_standing_on() {
        let expected = if cfg!(target_os = "windows") {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "this platform"
        };
        assert_eq!(GpuiClipboard.platform(), expected);
    }

    #[test]
    fn outside_a_frame_both_verbs_refuse_with_the_same_explanation() {
        // A test thread is never inside a gpui publication: this must fail, not touch a null `&mut App`.
        let e = GpuiClipboard.read_text().unwrap_err();
        assert!(e.contains("inside a frame"), "{e}");
        assert_eq!(GpuiClipboard.write_text("x").unwrap_err(), e);
    }

    #[test]
    fn install_is_idempotent_and_leaves_a_host_behind() {
        // `set_host` is last-writer-wins; a repeated boot must still end with a host.
        assert!(api::host().is_none(), "nothing installed a host before this test");
        install();
        install();
        let h = api::host().expect("a host is installed");
        assert_eq!(h.platform(), GpuiClipboard.platform());
    }
}
