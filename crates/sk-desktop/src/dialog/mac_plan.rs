//! What the macOS panels are told, decided in plain Rust so it is unit-tested on every OS.
//!
//! An `NSSavePanel` has no type list: `allowedContentTypes` is the filter and a "Format:" pop-up switches
//! it, with the Windows type list's entries in order and "All Files" last. The start folder ports
//! `SetFolder` / `SetDefaultFolder` and the client GUID's remembered folder: a forced `startIn` wins, else
//! the folder this `id` was last left in, else the default `startIn`.

use std::path::{Path, PathBuf};

use super::{Kind, Known, Native};

pub const ALL_FILES_MAC: &str = "All Files";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Format {
    pub name: String,
    pub exts: Option<Vec<String>>,
}

/// The pop-up's entries, the selected one first. Empty: no pop-up and no restriction; one entry: that
/// restriction with no pop-up. Names are made unique (`NSPopUpButton` merges equal titles).
pub fn formats(n: &Native) -> Vec<Format> {
    if n.kind == Kind::Folder || n.filters.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Format> = Vec::new();
    for f in &n.filters {
        let mut name = f.name.clone();
        let mut i = 2;
        while out.iter().any(|o| o.name == name) || name == ALL_FILES_MAC {
            name = format!("{} ({i})", f.name);
            i += 1;
        }
        out.push(Format { name, exts: Some(f.exts.clone()) });
    }
    if n.allow_all {
        out.push(Format { name: ALL_FILES_MAC.to_string(), exts: None });
    }
    out
}

/// Where the panel opens. `force` is `startIn` given; `remembered` is the folder this picker's `id` was left in.
pub fn start_folder(start: Option<PathBuf>, force: bool, remembered: Option<PathBuf>) -> Option<PathBuf> {
    if force && start.is_some() {
        return start;
    }
    remembered.or(start)
}

pub fn remembered_key(client: u128) -> String {
    format!("VueNativeDialogFolder.{client:032x}")
}

pub const FIRST_BUTTON: isize = 1000;

/// An `NSAlert` response → the index of the button pressed; anything else (the owner closing) is a dismissal.
pub fn button_of(response: isize, count: usize) -> Option<usize> {
    let i = response.checked_sub(FIRST_BUTTON)?;
    (i >= 0 && (i as usize) < count).then_some(i as usize)
}

pub fn rgb_of(hex: &str) -> (f64, f64, f64) {
    let n = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    let c = |shift: u32| f64::from((n >> shift) & 0xff) / 255.0;
    (c(16), c(8), c(0))
}

/// sRGB components → `#rrggbb`, clamped (wide-gamut pickers can fall outside 0..=1).
pub fn hex_of(r: f64, g: f64, b: f64) -> String {
    let c = |v: f64| if v.is_nan() { 0 } else { (v.clamp(0.0, 1.0) * 255.0).round() as u8 };
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

/// A known folder under the home folder, when `NSFileManager` has none: the names are fixed on disk.
pub fn home_folder(home: &Path, k: Known) -> PathBuf {
    home.join(match k {
        Known::Desktop => "Desktop",
        Known::Documents => "Documents",
        Known::Downloads => "Downloads",
        Known::Music => "Music",
        Known::Pictures => "Pictures",
        Known::Videos => "Movies",
    })
}

#[cfg(test)]
mod tests {
    use super::super::{Filter, Start};
    use super::*;

    fn native(kind: Kind, filters: &[(&str, &[&str])], allow_all: bool) -> Native {
        Native {
            kind,
            multiple: false,
            filters: filters
                .iter()
                .map(|(n, e)| Filter { name: n.to_string(), exts: e.iter().map(|e| e.to_string()).collect() })
                .collect(),
            allow_all,
            client: 0,
            start: Some(Start::Known(Known::Documents)),
            force_start: false,
            suggested_name: None,
            default_ext: None,
        }
    }

    #[test]
    fn formats_follow_the_windows_type_list_with_all_files_last() {
        let n = native(Kind::Open, &[("Image Files", &["png", "jpg"])], true);
        let f = formats(&n);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0], Format { name: "Image Files".into(), exts: Some(vec!["png".into(), "jpg".into()]) });
        assert_eq!(f[1], Format { name: ALL_FILES_MAC.into(), exts: None });
        let f = formats(&native(Kind::Save, &[("CSV", &["csv"])], false));
        assert_eq!(f, vec![Format { name: "CSV".into(), exts: Some(vec!["csv".into()]) }]);
        assert!(formats(&native(Kind::Open, &[], true)).is_empty());
        assert!(formats(&native(Kind::Folder, &[("Text", &["txt"])], true)).is_empty());
    }

    #[test]
    fn format_names_are_unique_for_the_popup() {
        let n = native(Kind::Open, &[("Text", &["txt"]), ("Text", &["log"]), ("All Files", &["md"])], true);
        let names: Vec<String> = formats(&n).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["Text", "Text (2)", "All Files (2)", "All Files"]);
    }

    #[test]
    fn the_start_folder_is_set_folder_or_set_default_folder() {
        let start = Some(PathBuf::from("/Users/me/Documents"));
        let remembered = Some(PathBuf::from("/Users/me/Projects"));
        assert_eq!(start_folder(start.clone(), true, remembered.clone()), start);
        assert_eq!(start_folder(start.clone(), false, remembered.clone()), remembered);
        assert_eq!(start_folder(start.clone(), false, None), start);
        assert_eq!(start_folder(None, false, None), None);
        assert_eq!(start_folder(None, true, remembered.clone()), remembered);
        assert_eq!(remembered_key(0xab), "VueNativeDialogFolder.000000000000000000000000000000ab");
    }

    #[test]
    fn alert_responses_map_back_to_button_indices() {
        assert_eq!(button_of(1000, 2), Some(0));
        assert_eq!(button_of(1001, 2), Some(1));
        assert_eq!(button_of(1002, 2), None);
        for r in [0, -1001, -1000, 999] {
            assert_eq!(button_of(r, 2), None, "{r}");
        }
    }

    #[test]
    fn colours_round_trip_through_srgb_components() {
        let (r, g, b) = rgb_of("#ff8000");
        assert_eq!((r, g, b), (1.0, 128.0 / 255.0, 0.0));
        assert_eq!(hex_of(r, g, b), "#ff8000");
        assert_eq!(hex_of(1.2, -0.1, f64::NAN), "#ff0000");
        assert_eq!(hex_of(0.5, 0.5, 0.5), "#808080");
        for hex in ["#000000", "#ffffff", "#12abef"] {
            let (r, g, b) = rgb_of(hex);
            assert_eq!(hex_of(r, g, b), hex);
        }
    }

    #[test]
    fn known_folders_fall_back_to_the_home_folder_names() {
        let home = Path::new("/Users/me");
        assert_eq!(home_folder(home, Known::Videos), Path::new("/Users/me/Movies"));
        assert_eq!(home_folder(home, Known::Downloads), Path::new("/Users/me/Downloads"));
    }
}
