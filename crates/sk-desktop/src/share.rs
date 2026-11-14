//! `navigator.share` through the system share sheet. Shared files are written to a per-process temp
//! folder, emptied at the next share.

use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "macos")]
pub use mac::start;
#[cfg(target_os = "windows")]
pub use win::start;

pub struct SharedFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub struct Request {
    pub title: String,
    pub text: Option<String>,
    pub url: Option<String>,
    pub files: Vec<SharedFile>,
}

fn folder() -> PathBuf {
    std::env::temp_dir().join(format!("sk-share-{}", std::process::id()))
}

/// A name that stays inside the folder.
fn safe_name(name: &str, i: usize) -> String {
    let clean: String = name.chars().filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    let clean = clean.trim().trim_matches('.');
    if clean.is_empty() {
        format!("file-{i}")
    } else {
        clean.to_string()
    }
}

/// The files written out, in order, each in a folder of its own so names may repeat.
fn write(files: Vec<SharedFile>) -> std::io::Result<Vec<PathBuf>> {
    let dir = folder();
    let _ = std::fs::remove_dir_all(&dir);
    files
        .into_iter()
        .enumerate()
        .map(|(i, f)| {
            let sub = dir.join(i.to_string());
            let path = sub.join(safe_name(&f.name, i));
            std::fs::create_dir_all(&sub)?;
            std::fs::write(&path, &f.bytes)?;
            Ok(path)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_names_stay_in_the_folder() {
        assert_eq!(super::safe_name("photo.png", 0), "photo.png");
        assert_eq!(super::safe_name("../../evil.exe", 1), "evil.exe");
        assert_eq!(super::safe_name("a\\b:c", 2), "abc");
        assert_eq!(super::safe_name("..", 3), "file-3");
    }
}
