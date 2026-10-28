//! `VN_DIALOGS=scripted`: a dialog that waits for the devtools instead of the
//! user (mod.rs, "the test seam").
//!
//! A scripted dialog runs on its own `vn-dialog` thread exactly like a real
//! one, owned and serialised the same way; only the middle step differs. The
//! thread parks a [`Pending`] entry and waits for `dialog answer` or `dialog
//! cancel`, checking its owner every 25 ms so a closing window cancels it the
//! way the `WM_CLOSE` timer cancels a real one.
//!
//! The answer is validated the way the OS dialog's flags would constrain the
//! user: an Open names existing files (`FOS_FILEMUSTEXIST`), at most one
//! without `multiple`; a folder pick names a folder; a Save names a path in
//! an existing folder (`FOS_PATHMUSTEXIST`). A rejected answer leaves the
//! dialog waiting, as a real dialog stays open on a bad entry.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value as Json};

use super::{describe, Cancellation, Kind, Native};
use crate::Fail;

/// The answer a scripted dialog waits for.
pub enum Answer {
    Paths(Vec<PathBuf>),
    Cancel,
}

struct Pending {
    id: u64,
    window: u64,
    what: &'static str,
    kind: Kind,
    multiple: bool,
    summary: Json,
    tx: Sender<Answer>,
}

static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
static NEXT: AtomicU64 = AtomicU64::new(1);

fn remove(id: u64) -> Option<Pending> {
    let mut p = PENDING.lock().unwrap_or_else(|p| p.into_inner());
    let i = p.iter().position(|x| x.id == id)?;
    Some(p.remove(i))
}

/// The dialog thread's side: park, then wait for an answer or the owner.
pub(super) fn pick(n: &Native, window: u64, what: &'static str, cancel: &Cancellation) -> Result<Option<Vec<PathBuf>>, Fail> {
    let (tx, rx) = channel();
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let summary = describe(n);
    crate::log_later(
        "dialog",
        format!("[dialog] #{id} {what} ({}) is waiting for a scripted answer (VN_DIALOGS=scripted)", n.kind.as_str()),
    );
    PENDING.lock().unwrap_or_else(|p| p.into_inner()).push(Pending {
        id,
        window,
        what,
        kind: n.kind,
        multiple: n.multiple,
        summary,
        tx,
    });
    loop {
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(Answer::Paths(p)) => return Ok(Some(p)),
            Ok(Answer::Cancel) | Err(RecvTimeoutError::Disconnected) => return Ok(None),
            Err(RecvTimeoutError::Timeout) => {
                if cancel.is_requested() {
                    remove(id);
                    crate::log_later("dialog", format!("[dialog] #{id} was closed with its window"));
                    return Ok(None);
                }
            }
        }
    }
}

/// `dialog list`: every dialog waiting for an answer, oldest first.
pub fn list() -> Json {
    let p = PENDING.lock().unwrap_or_else(|p| p.into_inner());
    Json::Array(
        p.iter()
            .map(|x| {
                let mut v = x.summary.clone();
                v["id"] = json!(x.id);
                v["window"] = json!(x.window);
                v["what"] = json!(x.what);
                v
            })
            .collect(),
    )
}

/// Would the OS dialog have let the user finish with these paths?
pub fn validate(kind: Kind, multiple: bool, paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("an answer needs at least one path (use `cancel` to dismiss)".into());
    }
    if paths.len() > 1 && !(kind == Kind::Open && multiple) {
        return Err(format!("this {} dialog takes one path, not {}", kind.as_str(), paths.len()));
    }
    for p in paths {
        if !p.is_absolute() {
            return Err(format!("{} is not an absolute path", p.display()));
        }
        let ok = match kind {
            Kind::Open => p.is_file(),
            Kind::Folder => p.is_dir(),
            Kind::Save => p.parent().is_some_and(|d| d.is_dir()) && !p.is_dir(),
        };
        if !ok {
            return Err(match kind {
                Kind::Open => format!("{} is not an existing file", p.display()),
                Kind::Folder => format!("{} is not an existing folder", p.display()),
                Kind::Save => format!("{} is not a file path in an existing folder", p.display()),
            });
        }
    }
    Ok(())
}

/// `dialog answer`: finish dialog `id` with `paths`.
pub fn answer(id: u64, paths: Vec<PathBuf>) -> Result<Json, String> {
    let mut pending = PENDING.lock().unwrap_or_else(|p| p.into_inner());
    let i = pending.iter().position(|x| x.id == id).ok_or_else(|| format!("no dialog #{id} is waiting"))?;
    validate(pending[i].kind, pending[i].multiple, &paths)?;
    let p = pending.remove(i);
    drop(pending);
    let n = paths.len();
    p.tx.send(Answer::Paths(paths)).map_err(|_| format!("dialog #{id} is gone"))?;
    crate::log_later("dialog", format!("[dialog] #{id} answered with {n} path(s)"));
    Ok(json!({ "id": id, "answered": n }))
}

/// `dialog cancel`: dismiss dialog `id`, as Esc would.
pub fn cancel(id: u64) -> Result<Json, String> {
    let p = remove(id).ok_or_else(|| format!("no dialog #{id} is waiting"))?;
    let _ = p.tx.send(Answer::Cancel);
    crate::log_later("dialog", format!("[dialog] #{id} cancelled"));
    Ok(json!({ "id": id, "cancelled": true }))
}
