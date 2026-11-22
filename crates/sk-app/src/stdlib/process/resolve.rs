//! Which program a call may start, with which arguments, in which environment. Everything is
//! decided here, against `"permissions.process"`, before the system is asked for anything.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use crate::permissions::{Permissions, ProcessArg, ProcessArgs, ProcessEntry};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    NotAllowed(String),
    NotFound(String),
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn example(name: &str) -> String {
    let name = if is_name(name) { name } else { "tool" };
    format!(
        r#""permissions": {{ "process": [{{ "name": "{name}", "path": "$APP/bin/{name}" }}] }}"#
    )
}

fn with_exe(path: PathBuf) -> PathBuf {
    if cfg!(windows) && path.extension().is_none() {
        path.with_extension("exe")
    } else {
        path
    }
}

/// `\\?\C:\x` is `C:\x`, and `\\?\UNC\host\share` is `\\host\share`; other paths stay.
pub(super) fn plain(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let Some(s) = path.to_str() else { return path };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// Windows file names compare without case.
pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// The file itself: links followed, `.` and `..` gone, and on Windows the long name in the case
/// on disk, without trailing dots or spaces.
pub(super) fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(plain)
}

#[cfg(windows)]
fn system_dir() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    let mut buf = [0u16; 260];
    // SAFETY: the buffer is as long as said.
    let n = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            buf.as_mut_ptr(),
            buf.len() as u32,
        )
    } as usize;
    (n > 0 && n < buf.len()).then(|| OsString::from_wide(&buf[..n]).into())
}

fn system_dirs() -> Vec<PathBuf> {
    #[cfg(windows)]
    return system_dir().into_iter().collect();
    #[cfg(not(windows))]
    return ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
        .iter()
        .map(PathBuf::from)
        .collect();
}

/// Absolute folders only: an empty or relative entry would search the current folder.
fn path_dirs(path: Option<&OsStr>) -> Vec<PathBuf> {
    path.map(|p| {
        std::env::split_paths(p)
            .filter(|d| d.is_absolute())
            .collect()
    })
    .unwrap_or_default()
}

/// The file an entry's `path` names, before it is made canonical.
pub(super) fn locate(
    path: &str,
    app_dir: &Path,
    search: Option<&OsStr>,
) -> Result<PathBuf, String> {
    let first = |dirs: Vec<PathBuf>, name: &str| {
        dirs.into_iter()
            .map(|d| with_exe(d.join(name)))
            .find(|f| f.is_file())
            .ok_or_else(|| format!("{path} is not there"))
    };
    if let Some(name) = path.strip_prefix("$SYSTEM/") {
        return if is_name(name) {
            first(system_dirs(), name)
        } else {
            Err(format!("{path:?} must be $SYSTEM/<a program name>"))
        };
    }
    if let Some(name) = path.strip_prefix("$PATH/") {
        return if is_name(name) {
            first(path_dirs(search), name)
        } else {
            Err(format!("{path:?} must be $PATH/<a program name>"))
        };
    }
    let file = if let Some(inside) = path.strip_prefix("$APP/") {
        let inside = Path::new(inside);
        let stays = inside
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
        if !stays || inside.file_name().is_none() {
            return Err(format!("{path:?} must be $APP/<a path inside the app>"));
        }
        with_exe(app_dir.join(inside))
    } else if Path::new(path).is_absolute() && !path.starts_with(r"\\") && !path.starts_with("//") {
        with_exe(PathBuf::from(path))
    } else {
        return Err(format!(
            "{path:?} must be $APP/<path>, $SYSTEM/<name>, $PATH/<name> or an absolute path"
        ));
    };
    if file.is_file() {
        Ok(file)
    } else {
        Err(format!("{} is not there", file.display()))
    }
}

/// Batch files are refused: CreateProcessW runs them through cmd.exe, which parses the arguments again.
fn runnable(path: &Path) -> Result<(), String> {
    let exe = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    if cfg!(windows) && !exe {
        return Err(format!(
            "{} is not an .exe; only .exe files are started, never batch files or scripts",
            path.display()
        ));
    }
    Ok(())
}

pub(super) fn args_allowed(rule: &ProcessArgs, args: &[String]) -> Result<(), String> {
    match rule {
        ProcessArgs::Any => Ok(()),
        ProcessArgs::None if args.is_empty() => Ok(()),
        ProcessArgs::None => Err("the entry takes no arguments; list them under \"args\"".into()),
        ProcessArgs::Only(rules) => {
            if rules.len() != args.len() {
                return Err(format!(
                    "the entry takes exactly {} argument(s), and {} were given",
                    rules.len(),
                    args.len()
                ));
            }
            for (i, (rule, arg)) in rules.iter().zip(args).enumerate() {
                let fits = match rule {
                    ProcessArg::Is(s) => s == arg,
                    ProcessArg::Matches(p) => regex::Regex::new(&format!(r"\A(?:{p})\z"))
                        .map_err(|_| {
                            format!(
                                "the pattern of argument {} is not a valid regular expression",
                                i + 1
                            )
                        })?
                        .is_match(arg),
                };
                if !fits {
                    return Err(format!("argument {} is not what the entry allows", i + 1));
                }
            }
            Ok(())
        }
    }
}

/// The canonical program to start for `program`: an entry's name, or an absolute path an entry
/// resolves to.
pub(super) fn resolve(
    permissions: &Permissions,
    program: &str,
    args: &[String],
    search: Option<&OsStr>,
) -> Result<PathBuf, Refusal> {
    if permissions.process.is_empty() {
        return Err(Refusal::NotAllowed(format!(
            "skathi:process may start only the programs sk.config.json lists, and it lists none; add {}",
            example(program)
        )));
    }
    let resolved = |e: &ProcessEntry| -> Result<PathBuf, Refusal> {
        let at = |why: String| format!("\"{}\" of \"permissions.process\": {why}", e.name);
        let file = locate(&e.path, &permissions.app_dir, search)
            .map_err(|why| Refusal::NotFound(at(why)))?;
        let file = canonical(&file)
            .map_err(|err| Refusal::NotFound(at(format!("{}: {err}", file.display()))))?;
        runnable(&file).map_err(|why| Refusal::NotAllowed(at(why)))?;
        Ok(file)
    };
    let mut candidates: Vec<(&ProcessEntry, PathBuf)> = Vec::new();
    let mut missing = None;
    if is_name(program) {
        for e in permissions.process.iter().filter(|e| e.name == program) {
            match resolved(e) {
                Ok(file) => candidates.push((e, file)),
                Err(refusal) => missing = Some(refusal),
            }
        }
        if candidates.is_empty() {
            return Err(match missing {
                Some(refusal) => refusal,
                None => Refusal::NotAllowed(format!(
                    "the program \"{program}\" is not in \"permissions.process\" of sk.config.json; add {}",
                    example(program)
                )),
            });
        }
    } else if Path::new(program).is_absolute() {
        let target = canonical(Path::new(program))
            .map_err(|e| Refusal::NotFound(format!("{program}: {e}")))?;
        for e in &permissions.process {
            if let Ok(file) = resolved(e) {
                if same_path(&file, &target) {
                    candidates.push((e, file));
                }
            }
        }
        if candidates.is_empty() {
            return Err(Refusal::NotAllowed(format!(
                "{program} is not a program \"permissions.process\" of sk.config.json lists; add an entry for it and start it by its name"
            )));
        }
    } else {
        return Err(Refusal::NotAllowed(format!(
            "{program:?} is neither a name from \"permissions.process\" nor an absolute path"
        )));
    }
    let mut why = String::new();
    for (e, file) in candidates {
        match args_allowed(&e.args, args) {
            Ok(()) => return Ok(file),
            Err(w) => why = format!("\"{}\" of \"permissions.process\": {w}", e.name),
        }
    }
    Err(Refusal::NotAllowed(why))
}

/// Variables that decide what the child loads or runs before its own code: app code cannot set them.
pub(super) fn forbidden_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    [
        "PATH",
        "PATHEXT",
        "COMSPEC",
        "SYSTEMROOT",
        "WINDIR",
        "IFS",
        "ENV",
        "BASH_ENV",
        "SHELLOPTS",
        "BASHOPTS",
        "PS4",
    ]
    .contains(&key.as_str())
        || ["LD_", "DYLD_", "BASH_FUNC_"]
            .iter()
            .any(|p| key.starts_with(p))
}

fn same_key(a: &OsStr, b: &OsStr) -> bool {
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

/// AppImage's runtime points these at the image; a child is not part of it.
const APPIMAGE_VARS: [&str; 4] = ["APPIMAGE", "APPDIR", "ARGV0", "OWD"];

/// The host's environment without Skathi's own `SK_*` settings and, inside an AppImage
/// (`appdir`), without the image's folders; then the app's `overrides` over it.
pub(super) fn child_env(
    host: impl IntoIterator<Item = (OsString, OsString)>,
    overrides: &[(String, String)],
    appdir: Option<&str>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = Vec::new();
    for (k, v) in host {
        let key = k.to_string_lossy();
        if key.to_ascii_uppercase().starts_with("SK_") {
            continue;
        }
        let Some(appdir) = appdir.filter(|d| !d.is_empty()) else {
            env.push((k, v));
            continue;
        };
        if APPIMAGE_VARS.contains(&key.as_ref()) {
            continue;
        }
        match v.to_str() {
            Some(value) if value.contains(appdir) => {
                let kept: Vec<&str> = value
                    .split(':')
                    .filter(|part| !part.starts_with(appdir))
                    .collect();
                if !kept.is_empty() && kept.iter().any(|p| !p.is_empty()) {
                    env.push((k, kept.join(":").into()));
                }
            }
            _ => env.push((k, v)),
        }
    }
    for (k, v) in overrides {
        let k = OsString::from(k);
        env.retain(|(have, _)| !same_key(have, &k));
        env.push((k, v.into()));
    }
    env
}
