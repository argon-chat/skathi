//! Trusted, per-user cache of packaged classic-script bytecode. Source remains
//! authoritative; a cache miss never prevents the application from starting.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use vn_runtime::redjs::api::{Bytecode, Error};
use vn_runtime::{runtime::JsError, JsRuntime};

const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;
const SCRIPT_URL: &str = "vn://app.prod.js";
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub fn eval(rt: &JsRuntime, source: &str) -> Result<(), JsError> {
    let Some(folder) = cache_folder() else {
        return crate::perf::boot_eval("app.prod.js (+mount)", || rt.eval(source, SCRIPT_URL));
    };
    let (key, cached) = crate::perf::boot_eval("bytecode lookup", || {
        let key = rt.ctx().bytecode_cache_key(source, SCRIPT_URL);
        let file = folder.join(cache_name(&key));
        let cached = read_bounded(&file).ok().flatten().and_then(|bytes| {
            // This directory is local to the same trusted OS user who can
            // replace this executable. Never load downloads or arbitrary app
            // assets here: JSC's decoder is not an untrusted-input validator.
            unsafe { Bytecode::from_trusted_bytes(&bytes, key) }
        });
        (key, cached)
    });
    let cache = match cached {
        Some(cache) => Some(cache),
        None => {
            let generated = crate::perf::boot_eval("bytecode generate", || {
                // Production bundles have an outer IIFE: include its body,
                // while keeping deeper functions lazy for a bounded cold cost.
                rt.ctx().compile_bytecode_with_depth(source, SCRIPT_URL, 1)
            });
            match generated {
                Ok(cache) => {
                    if cache.as_bytes().len() as u64 <= MAX_CACHE_BYTES {
                        let result = crate::perf::boot_eval("bytecode write", || {
                            write_cache(&folder, &cache_name(&key), cache.as_bytes())
                        });
                        if let Err(error) = result {
                            eprintln!("[bytecode] cannot save cache: {error}");
                        }
                    }
                    Some(cache)
                }
                Err(error) => {
                    let error = describe(rt, error);
                    eprintln!(
                        "[bytecode] generation failed; using source: {}",
                        error.header
                    );
                    None
                }
            }
        }
    };
    crate::perf::boot_eval("app.prod.js (+mount)", || {
        match rt
            .ctx()
            .eval_named_with_bytecode(source, SCRIPT_URL, cache.as_ref())
        {
            Ok((_, used)) => {
                eprintln!(
                    "[bytecode] {}",
                    if used { "JSC accepted cache" } else { "source" }
                );
                Ok(())
            }
            // An exception may follow observable JS side effects. Never retry.
            Err(error) => Err(describe(rt, error)),
        }
    })
}

fn describe(rt: &JsRuntime, error: Error) -> JsError {
    match error {
        Error::Exception(exception) => rt.describe(exception),
        other => JsError {
            header: format!("engine error: {other}"),
            stack: String::new(),
        },
    }
}

fn cache_name(key: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut name = String::with_capacity(67);
    for byte in key {
        write!(name, "{byte:02x}").expect("writing into a String cannot fail");
    }
    name.push_str(".bc");
    name
}

fn cache_folder() -> Option<PathBuf> {
    if std::env::var_os("VN_BYTECODE_CACHE").is_some_and(|value| value == "0") {
        return None;
    }
    if let Some(folder) =
        std::env::var_os("VN_BYTECODE_CACHE_DIR").filter(|value| !value.is_empty())
    {
        return Some(PathBuf::from(folder));
    }
    local_data_folder().map(|base| {
        base.join(crate::payload::slot_name())
            .join("cache")
            .join("jsc")
    })
}

#[cfg(windows)]
fn local_data_folder() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let wide =
        unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None) }.ok()?;
    let path = unsafe { wide.to_string() }.ok().map(PathBuf::from);
    unsafe { CoTaskMemFree(Some(wide.0.cast())) };
    path
}

#[cfg(not(windows))]
fn local_data_folder() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
}

fn read_bounded(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_CACHE_BYTES {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(MAX_CACHE_BYTES + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= MAX_CACHE_BYTES).then_some(bytes))
}

fn write_cache(folder: &Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(folder)?;
    let destination = folder.join(name);
    let temporary = folder.join(format!(
        ".{}-{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        drop(file);
        // A key is immutable. Another process winning this race is harmless;
        // remove only this exact cache entry when replacing a corrupt copy.
        if destination.is_file() {
            fs::remove_file(&destination)?;
        }
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if let Err(error) = fs::remove_file(&temporary) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("[bytecode] cannot remove {}: {error}", temporary.display());
        }
    }
    if result.is_ok() {
        prune_previous(folder, name);
    }
    result
}

fn prune_previous(folder: &Path, current: &str) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(hash) = name.strip_suffix(".bc") else {
            continue;
        };
        if name != current && hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            if let Err(error) = fs::remove_file(entry.path()) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("[bytecode] cannot prune old cache: {error}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_oversized_and_replaced_files_are_bounded() {
        let folder = std::env::temp_dir().join(format!(
            "vn-bytecode-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&folder).unwrap();
        let first = cache_name(&[1; 32]);
        let second = cache_name(&[2; 32]);
        assert_eq!(read_bounded(&folder.join(&first)).unwrap(), None);
        let file = File::create(folder.join(&first)).unwrap();
        file.set_len(MAX_CACHE_BYTES + 1).unwrap();
        drop(file);
        assert_eq!(read_bounded(&folder.join(&first)).unwrap(), None);
        write_cache(&folder, &first, b"old").unwrap();
        write_cache(&folder, &first, b"replacement").unwrap();
        assert_eq!(
            read_bounded(&folder.join(&first)).unwrap().unwrap(),
            b"replacement"
        );
        fs::write(folder.join("unrelated.bc"), b"keep").unwrap();
        write_cache(&folder, &second, b"new").unwrap();
        assert!(!folder.join(&first).exists());
        assert_eq!(fs::read(folder.join("unrelated.bc")).unwrap(), b"keep");
        for name in [second.as_str(), "unrelated.bc"] {
            fs::remove_file(folder.join(name)).unwrap();
        }
        fs::remove_dir(folder).unwrap();
    }
}
