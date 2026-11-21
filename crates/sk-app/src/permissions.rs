//! `"permissions"` in sk.config.json: what the app's code may reach outside the sandbox of the web
//! platform. The list is checked here, before the system is asked for anything.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use crate::json::{self, Json};

#[derive(Default, Debug)]
pub(crate) struct Permissions {
    /// A library's name in `dlopen()` and where it is.
    pub ffi: Vec<(String, String)>,
    /// Whether a pointer may be made from a bare address.
    pub ffi_address: bool,
    /// What `$APP` stands for.
    pub app_dir: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Library {
    File(PathBuf),
    /// A name the system's loader looks up in its own folders.
    System(String),
}

impl Permissions {
    pub fn parse(text: &str, app_dir: PathBuf) -> Permissions {
        let Some(v) = json::parse(text) else {
            return Permissions::default();
        };
        let entry = |e: &Json| {
            Some((
                e.get("name")?.as_str()?.to_string(),
                e.get("path")?.as_str()?.to_string(),
            ))
        };
        Permissions {
            ffi: v
                .get("ffi")
                .and_then(Json::as_arr)
                .map(|list| list.iter().filter_map(entry).collect())
                .unwrap_or_default(),
            ffi_address: v.get("ffiAddress").and_then(Json::as_bool) == Some(true),
            app_dir,
        }
    }

    pub fn library(&self, name: &str) -> Result<Library, String> {
        let Some((_, path)) = self.ffi.iter().find(|(n, _)| n == name) else {
            let example = format!(
                r#""permissions": {{ "ffi": [{{ "name": "{name}", "path": "$APP/lib/{name}" }}] }}"#
            );
            return Err(if self.ffi.is_empty() {
                format!("skathi:ffi may load only the libraries sk.config.json lists, and it lists none; add {example}")
            } else {
                format!("the library \"{name}\" is not in \"permissions.ffi\" of sk.config.json; add {example}")
            });
        };
        locate(path, &self.app_dir)
            .ok_or_else(|| format!("\"permissions.ffi\": the path of \"{name}\", {path:?}, must be $APP/<a path inside the app> or $SYSTEM/<a library name>"))
    }
}

fn locate(path: &str, app_dir: &Path) -> Option<Library> {
    if let Some(name) = path.strip_prefix("$SYSTEM/") {
        let plain = !name.is_empty() && !name.contains(['/', '\\', ':']) && !name.starts_with('.');
        return plain.then(|| Library::System(system_name(name)));
    }
    let inside = Path::new(path.strip_prefix("$APP/")?);
    let stays = inside
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !stays || inside.file_name().is_none() {
        return None;
    }
    let candidates = candidates(&app_dir.join(inside));
    let found = candidates.iter().find(|c| c.is_file());
    Some(Library::File(found.unwrap_or(&candidates[0]).clone()))
}

const EXTENSION: &str = if cfg!(windows) {
    "dll"
} else if cfg!(target_vendor = "apple") {
    "dylib"
} else {
    "so"
};

fn system_name(name: &str) -> String {
    if cfg!(windows) && Path::new(name).extension().is_none() {
        format!("{name}.dll")
    } else {
        name.to_string()
    }
}

/// `lib/sqlite3` is `lib/sqlite3.dll` on Windows, and `lib/sqlite3.so` or `lib/libsqlite3.so` elsewhere.
fn candidates(path: &Path) -> Vec<PathBuf> {
    if path.extension().is_some() {
        return vec![path.to_path_buf()];
    }
    let mut out = vec![path.with_extension(EXTENSION)];
    if !cfg!(windows) {
        if let Some(file) = path.file_name().and_then(|f| f.to_str()) {
            out.push(path.with_file_name(format!("lib{file}.{EXTENSION}")));
        }
    }
    out
}

static CURRENT: OnceLock<Permissions> = OnceLock::new();

/// An embedded app has what its embedder allowed and nothing from the payload: the process is the embedder's.
pub(crate) fn current() -> &'static Permissions {
    CURRENT.get_or_init(|| {
        if crate::embed::active() {
            return crate::embed::permissions();
        }
        match crate::boot_mode::permissions() {
            Some((text, app_dir)) => Permissions::parse(&text, app_dir),
            None => Permissions::default(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(path: &str) -> Permissions {
        let text = format!(r#"{{"ffi":[{{"name":"db","path":"{path}"}}]}}"#);
        Permissions::parse(&text, std::env::temp_dir().join("sk-permissions-test"))
    }

    #[test]
    fn a_library_is_found_by_its_listed_name_only() {
        let p = allowed("$APP/lib/db");
        let expected = p.app_dir.join("lib").join(format!("db.{EXTENSION}"));
        assert_eq!(p.library("db"), Ok(Library::File(expected)));
        assert!(p.library("other").unwrap_err().contains("\"other\""));
        assert!(Permissions::default()
            .library("db")
            .unwrap_err()
            .contains("lists none"));
    }

    #[test]
    fn a_path_cannot_leave_the_app_or_name_a_folder_of_the_system() {
        for path in [
            "$APP/../db",
            "$APP//etc/db",
            "$APP/",
            "C:/Windows/db.dll",
            "db",
            "$SYSTEM/../db",
            "$SYSTEM/sub/db",
            "$SYSTEM/",
        ] {
            assert!(allowed(path).library("db").is_err(), "{path}");
        }
    }

    #[test]
    fn a_system_library_is_a_bare_name() {
        let expected = if cfg!(windows) {
            "user32.dll"
        } else {
            "user32"
        };
        assert_eq!(
            allowed("$SYSTEM/user32").library("db"),
            Ok(Library::System(expected.into()))
        );
    }

    #[test]
    fn making_pointers_from_addresses_is_its_own_permission() {
        assert!(!allowed("$APP/db").ffi_address);
        let p = Permissions::parse(r#"{"ffi":[],"ffiAddress":true}"#, PathBuf::new());
        assert!(p.ffi_address);
    }
}
