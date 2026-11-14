//! Linux: what every desktop answers, with no toolkit of our own. File pickers use the desktop portal's
//! FileChooser (ashpd), which has no owner here (gpui does not hand out window handles). A question or
//! colour uses `zenity`, or `kdialog` on KDE. Each runs on the dialog's own thread and blocks it. A helper
//! process is killed when the owner closes; a portal request cannot be closed before it answers, so the
//! promise settles as a cancel and the late result is ignored.

use std::future::Future;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};

use super::*;

pub(super) fn pick(n: &Native, _: isize, cancel: &Cancellation) -> Result<Option<Vec<PathBuf>>, Fail> {
    if cancel.is_requested() {
        return Ok(None);
    }
    let filters = portal_filters(n);
    let folder = start_folder(n);
    let request = async {
        match n.kind {
            Kind::Save => {
                SelectedFiles::save_file()
                    .modal(true)
                    .current_name(n.suggested_name.as_deref())
                    .filters(filters)
                    .current_folder::<&Path>(folder.as_deref())?
                    .send()
                    .await
            }
            Kind::Open | Kind::Folder => {
                SelectedFiles::open_file()
                    .modal(true)
                    .multiple(n.kind == Kind::Open && n.multiple)
                    .directory(n.kind == Kind::Folder)
                    .filters(filters)
                    .current_folder::<&Path>(folder.as_deref())?
                    .send()
                    .await
            }
        }
    };
    let Some(sent) = until_cancelled(cancel, request) else { return Ok(None) };
    let selected = match sent.and_then(|r| r.response()) {
        Ok(selected) => selected,
        Err(ashpd::Error::Response(_)) => return Ok(None),
        Err(ashpd::Error::PortalNotFound(_)) => {
            return Err(Fail::new(
                "NotSupportedError",
                "file dialogs need xdg-desktop-portal and a portal backend (GNOME, KDE, GTK or wlroots)",
            )
            .code("ENOSYS"))
        }
        Err(e) => return Err(Fail::new("InvalidStateError", format!("the file dialog failed: {e}")).code("EIO")),
    };
    let mut paths: Vec<PathBuf> = selected.uris().iter().filter_map(|u| file_path_of(u.as_str())).collect();
    if paths.is_empty() {
        return Ok(None);
    }
    if n.kind == Kind::Save {
        paths.truncate(1);
        if let (Some(ext), Some(path)) = (n.default_ext.as_deref(), paths.first_mut()) {
            with_default_ext(path, ext);
        }
    }
    Ok(Some(paths))
}

fn portal_filters(n: &Native) -> Vec<FileFilter> {
    let mut out: Vec<FileFilter> = n
        .filters
        .iter()
        .map(|f| f.exts.iter().fold(FileFilter::new(&f.name), |filter, e| filter.glob(&caseless_glob(e))))
        .collect();
    if n.allow_all || out.is_empty() {
        out.push(FileFilter::new(ALL_FILES).glob("*"));
    }
    out
}

fn caseless_glob(ext: &str) -> String {
    let mut glob = String::from("*.");
    for c in ext.chars() {
        if c.is_ascii_alphabetic() {
            glob.push('[');
            glob.push(c.to_ascii_lowercase());
            glob.push(c.to_ascii_uppercase());
            glob.push(']');
        } else if matches!(c, '[' | ']' | '*' | '?') {
            glob.push('[');
            glob.push(c);
            glob.push(']');
        } else {
            glob.push(c);
        }
    }
    glob
}

fn start_folder(n: &Native) -> Option<PathBuf> {
    match n.start.as_ref()? {
        Start::Known(k) => known_folder(*k),
        Start::Path(p) if p.is_dir() => Some(p.clone()),
        Start::Path(p) => p.parent().filter(|d| d.is_dir()).map(Path::to_path_buf),
    }
}

/// The save picker's `SetDefaultExtension`: a name typed without one gets the type's.
fn with_default_ext(path: &mut PathBuf, ext: &str) {
    if path.extension().is_some() || ext.is_empty() {
        return;
    }
    let extended = path.with_extension(ext);
    if !extended.exists() {
        *path = extended;
    }
}

fn file_path_of(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let rest = uri.strip_prefix("file://")?;
    let path = match rest.find('/') {
        Some(0) => rest,
        Some(i) if &rest[..i] == "localhost" => &rest[i..],
        _ => return None,
    };
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

fn until_cancelled<T>(cancel: &Cancellation, fut: impl Future<Output = T>) -> Option<T> {
    async_io::block_on(futures_lite::future::or(async { Some(fut.await) }, async {
        loop {
            async_io::Timer::after(Duration::from_millis(100)).await;
            if cancel.is_requested() {
                return None;
            }
        }
    }))
}

pub(super) fn known_folder(k: Known) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute())?;
    let (key, default) = match k {
        Known::Desktop => ("XDG_DESKTOP_DIR", "Desktop"),
        Known::Documents => ("XDG_DOCUMENTS_DIR", "Documents"),
        Known::Downloads => ("XDG_DOWNLOAD_DIR", "Downloads"),
        Known::Music => ("XDG_MUSIC_DIR", "Music"),
        Known::Pictures => ("XDG_PICTURES_DIR", "Pictures"),
        Known::Videos => ("XDG_VIDEOS_DIR", "Videos"),
    };
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|c| c.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let saved = std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|dirs| user_dir(&dirs, key, &home));
    Some(saved.unwrap_or_else(|| home.join(default)))
}

fn user_dir(dirs: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let value = dirs.lines().find_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))?;
    let value = value.trim().strip_prefix('"')?.strip_suffix('"')?;
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None if value.starts_with('/') => PathBuf::from(value),
        None => return None,
    };
    (path != home).then_some(path)
}

fn kde_first() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.split(':').any(|d| d.eq_ignore_ascii_case("KDE")))
}

type Said = (ExitStatus, String);

/// Runs a helper to completion, killing it if the owner starts to close (`Ok(None)`).
fn run_watched(mut cmd: Command, cancel: &Cancellation) -> std::io::Result<Option<Said>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    loop {
        if let Some(status) = child.try_wait()? {
            let mut out = String::new();
            if let Some(mut stdout) = child.stdout.take() {
                let _ = stdout.read_to_string(&mut out);
            }
            return Ok(Some((status, out)));
        }
        if cancel.is_requested() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn ask_helpers<T>(
    cancel: &Cancellation,
    what: &str,
    helpers: Vec<(&'static str, Command, Box<dyn FnOnce(Said) -> T>)>,
) -> Result<Option<T>, Fail> {
    for (name, cmd, read) in helpers {
        match run_watched(cmd, cancel) {
            Ok(Some(said)) => return Ok(Some(read(said))),
            Ok(None) => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Fail::new("InvalidStateError", format!("{what}: {name} could not start: {e}")).code("EIO")),
        }
    }
    Err(Fail::new("NotSupportedError", format!("{what} needs zenity or kdialog")).code("ENOSYS"))
}

fn ordered<T>(mut helpers: Vec<(&'static str, Command, T)>) -> Vec<(&'static str, Command, T)> {
    if kde_first() {
        helpers.reverse();
    }
    helpers
}

/// The question: zenity's own Cancel, Esc and close box are a dismissal. kdialog: yes / no / cancel, up to three buttons.
pub(super) fn ask(q: &Ask, _: isize, cancel: &Cancellation) -> Result<Option<usize>, Fail> {
    if cancel.is_requested() {
        return Ok(None);
    }
    let labels: Vec<String> = q.buttons.iter().map(|b| ask::label_of(b)).collect();
    let text = if q.content.is_empty() { q.instruction.clone() } else { format!("{}\n\n{}", q.instruction, q.content) };

    let mut zenity = Command::new("zenity");
    zenity.args(["--question", "--no-markup", "--width=420"]);
    zenity.arg(format!("--title={}", q.title)).arg(format!("--text={text}"));
    zenity.arg(format!("--ok-label={}", labels[0]));
    for l in &labels[1..] {
        zenity.arg(format!("--extra-button={l}"));
    }
    if q.default != 0 {
        zenity.arg("--default-cancel");
    }
    let extra = labels.clone();
    let zenity_read: Box<dyn FnOnce(Said) -> Option<usize>> = Box::new(move |(status, out): Said| {
        if status.success() {
            return Some(0);
        }
        let pressed = out.trim_end_matches('\n');
        extra.iter().skip(1).position(|l| l == pressed).map(|i| i + 1)
    });
    let mut helpers = vec![("zenity", zenity, zenity_read)];

    if labels.len() <= 3 {
        let mut kdialog = Command::new("kdialog");
        kdialog.arg(format!("--title={}", q.title));
        kdialog.args(["--yesnocancel", &text, "--yes-label", &labels[0]]);
        if let Some(no) = labels.get(1) {
            kdialog.args(["--no-label", no]);
        }
        if let Some(third) = labels.get(2) {
            kdialog.args(["--cancel-label", third]);
        }
        let three = labels.len() == 3;
        let kdialog_read: Box<dyn FnOnce(Said) -> Option<usize>> = Box::new(move |(status, _): Said| match status.code() {
            Some(0) => Some(0),
            Some(1) => Some(1),
            Some(2) if three => Some(2),
            _ => None,
        });
        helpers.push(("kdialog", kdialog, kdialog_read));
    }
    ask_helpers(cancel, "native questions", ordered(helpers)).map(Option::flatten)
}

pub(super) fn color(value: &str, _: isize, cancel: &Cancellation) -> Result<Option<String>, Fail> {
    if cancel.is_requested() {
        return Ok(None);
    }
    let mut zenity = Command::new("zenity");
    zenity.args(["--color-selection", "--show-palette"]).arg(format!("--color={value}"));
    let mut kdialog = Command::new("kdialog");
    kdialog.args(["--getcolor", "--default", value]);
    let read: fn(Said) -> Option<String> = |(status, out)| status.success().then(|| hex_of(out.trim())).flatten();
    let helpers: Vec<(&'static str, Command, Box<dyn FnOnce(Said) -> Option<String>>)> =
        vec![("zenity", zenity, Box::new(read)), ("kdialog", kdialog, Box::new(read))];
    ask_helpers(cancel, "color pickers", ordered(helpers)).map(Option::flatten)
}

fn hex_of(s: &str) -> Option<String> {
    if let Some(hex) = s.strip_prefix('#') {
        return (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", hex.to_ascii_lowercase()));
    }
    let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
    let mut parts = inner.split(',').map(|p| p.trim().parse::<f64>().ok());
    let mut channel = || parts.next().flatten().map(|v| v.round().clamp(0., 255.) as u8);
    Some(format!("#{:02x}{:02x}{:02x}", channel()?, channel()?, channel()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_either_case() {
        assert_eq!(caseless_glob("png"), "*.[pP][nN][gG]");
        assert_eq!(caseless_glob("tar.gz"), "*.[tT][aA][rR].[gG][zZ]");
        assert_eq!(caseless_glob("mp4"), "*.[mM][pP]4");
    }

    #[test]
    fn file_uris_become_paths() {
        assert_eq!(file_path_of("file:///home/a%20b/%D1%84.txt"), Some(PathBuf::from("/home/a b/ф.txt")));
        assert_eq!(file_path_of("file://localhost/tmp/x"), Some(PathBuf::from("/tmp/x")));
        assert_eq!(file_path_of("file://elsewhere/tmp/x"), None);
        assert_eq!(file_path_of("https://example.com/x"), None);
        assert_eq!(file_path_of("file:///100%"), Some(PathBuf::from("/100%")));
    }

    #[test]
    fn user_dirs_are_read() {
        let home = Path::new("/home/u");
        let dirs = "# comment\nXDG_DOWNLOAD_DIR=\"$HOME/Загрузки\"\nXDG_MUSIC_DIR=\"/data/music\"\nXDG_VIDEOS_DIR=\"$HOME/\"\n";
        assert_eq!(user_dir(dirs, "XDG_DOWNLOAD_DIR", home), Some(PathBuf::from("/home/u/Загрузки")));
        assert_eq!(user_dir(dirs, "XDG_MUSIC_DIR", home), Some(PathBuf::from("/data/music")));
        assert_eq!(user_dir(dirs, "XDG_VIDEOS_DIR", home), None);
        assert_eq!(user_dir(dirs, "XDG_DESKTOP_DIR", home), None);
    }

    #[test]
    fn helper_colours_become_hex() {
        assert_eq!(hex_of("rgb(255,128,0)").as_deref(), Some("#ff8000"));
        assert_eq!(hex_of("rgba(1,2,3,0.5)").as_deref(), Some("#010203"));
        assert_eq!(hex_of("#A0B0C0").as_deref(), Some("#a0b0c0"));
        assert_eq!(hex_of("nope"), None);
    }
}
