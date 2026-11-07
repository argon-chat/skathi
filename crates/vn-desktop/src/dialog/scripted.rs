//! `VN_DIALOGS=scripted`: a dialog that waits for the devtools instead of the
//! user (mod.rs, "the test seam").
//!
//! A scripted dialog runs on its own `vn-dialog` (or `vn-ask`) thread exactly
//! like a real one, owned and serialised the same way; only the middle step
//! differs. The thread parks a [`Pending`] entry and waits for `dialog
//! answer` (a file dialog's paths), `dialog choose` (a question's button) or
//! `dialog cancel`, checking its owner every 25 ms so a closing window
//! cancels it the way the `WM_CLOSE` timer cancels a real one.
//!
//! The answer is validated the way the OS dialog's flags would constrain the
//! user: an Open names existing files (`FOS_FILEMUSTEXIST`), at most one
//! without `multiple`; a folder pick names a folder; a Save names a path in
//! an existing folder (`FOS_PATHMUSTEXIST`); a question takes one of its own
//! buttons. A rejected answer leaves the dialog waiting, as a real dialog
//! stays open on a bad entry.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value as Json};

use super::ask::{label_of, Ask};
use super::{describe, Cancellation, Kind, Native};
use crate::Fail;

/// The answer a scripted dialog waits for.
pub enum Answer {
    Paths(Vec<PathBuf>),
    Button(usize),
    Color(String),
    Cancel,
}

/// What a waiting dialog takes as an answer.
enum Expect {
    Color,
    /// A file dialog: paths, checked against its kind.
    Paths { kind: Kind, multiple: bool },
    /// A question: one of its buttons, by label (without `&`).
    Button { labels: Vec<String> },
}

struct Pending {
    id: u64,
    window: u64,
    what: &'static str,
    expect: Expect,
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

/// Parks a dialog and waits for its answer, or for its owner to close
/// (`None`, logged).
fn wait(window: u64, what: &'static str, expect: Expect, summary: Json, waiting: String, cancel: &Cancellation) -> Option<Answer> {
    let (tx, rx) = channel();
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    crate::log_later("dialog", format!("[dialog] #{id} {waiting} is waiting for a scripted answer (VN_DIALOGS=scripted)"));
    PENDING.lock().unwrap_or_else(|p| p.into_inner()).push(Pending { id, window, what, expect, summary, tx });
    loop {
        match rx.recv_timeout(Duration::from_millis(25)) {
            Ok(a) => return Some(a),
            Err(RecvTimeoutError::Disconnected) => return Some(Answer::Cancel),
            Err(RecvTimeoutError::Timeout) => {
                if cancel.is_requested() {
                    remove(id);
                    crate::log_later("dialog", format!("[dialog] #{id} was closed with its window"));
                    return None;
                }
            }
        }
    }
}

/// The dialog thread's side of a file dialog.
pub(super) fn pick(n: &Native, window: u64, what: &'static str, cancel: &Cancellation) -> Result<Option<Vec<PathBuf>>, Fail> {
    let expect = Expect::Paths { kind: n.kind, multiple: n.multiple };
    let waiting = format!("{what} ({})", n.kind.as_str());
    Ok(match wait(window, what, expect, describe(n), waiting, cancel) {
        Some(Answer::Paths(p)) => Some(p),
        _ => None,
    })
}

/// The dialog thread's side of a question (ask.rs): the button's index, or
/// `None` for a dismissal.
pub(super) fn ask(q: &Ask, window: u64, cancel: &Cancellation) -> Option<usize> {
    let labels: Vec<String> = q.buttons.iter().map(|b| label_of(b)).collect();
    let summary = json!({
        "kind": "ask",
        "title": q.title,
        "instruction": q.instruction,
        "content": q.content,
        "buttons": labels,
        "default": labels.get(q.default),
    });
    let waiting = format!("{} (ask: {})", q.what, labels.join(" / "));
    match wait(window, q.what, Expect::Button { labels }, summary, waiting, cancel) {
        Some(Answer::Button(i)) => Some(i),
        _ => None,
    }
}

pub(super) fn color(value: &str, window: u64, cancel: &Cancellation) -> Result<Option<String>, Fail> {
    Ok(match wait(window, "color", Expect::Color, json!({ "kind": "color", "value": value }), "color picker".into(), cancel) {
        Some(Answer::Color(color)) => Some(color),
        _ => None,
    })
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

/// `dialog answer`: finish file dialog `id` with `paths`.
pub fn answer(id: u64, paths: Vec<PathBuf>) -> Result<Json, String> {
    let mut pending = PENDING.lock().unwrap_or_else(|p| p.into_inner());
    let i = pending.iter().position(|x| x.id == id).ok_or_else(|| format!("no dialog #{id} is waiting"))?;
    match &pending[i].expect {
        Expect::Color => return Err(format!("dialog #{id} is a color picker: use `choose` with #rrggbb")),
        Expect::Paths { kind, multiple } => validate(*kind, *multiple, &paths)?,
        Expect::Button { labels } => {
            return Err(format!("dialog #{id} is a question: answer it with `choose` and one of {labels:?}"))
        }
    }
    let p = pending.remove(i);
    drop(pending);
    let n = paths.len();
    p.tx.send(Answer::Paths(paths)).map_err(|_| format!("dialog #{id} is gone"))?;
    crate::log_later("dialog", format!("[dialog] #{id} answered with {n} path(s)"));
    Ok(json!({ "id": id, "answered": n }))
}

/// Which of `labels` `button` names: an index, or a label (any case, `&`
/// ignored).
fn button_index(labels: &[String], button: &str) -> Option<usize> {
    let b = button.trim();
    if let Ok(i) = b.parse::<usize>() {
        return (i < labels.len()).then_some(i);
    }
    let b = label_of(b).to_lowercase();
    labels.iter().position(|l| l.to_lowercase() == b)
}

/// `dialog choose`: press `button` (an index or a label) in question `id`.
pub fn choose(id: u64, button: &str) -> Result<Json, String> {
    let mut pending = PENDING.lock().unwrap_or_else(|p| p.into_inner());
    let i = pending.iter().position(|x| x.id == id).ok_or_else(|| format!("no dialog #{id} is waiting"))?;
    if matches!(pending[i].expect, Expect::Color) {
        let color = super::color::normalize(button).ok_or("a color picker needs #rrggbb")?;
        let dialog = pending.remove(i);
        drop(pending);
        dialog.tx.send(Answer::Color(color.clone())).map_err(|_| format!("dialog #{id} is gone"))?;
        return Ok(json!({ "id": id, "color": color }));
    }
    let index = match &pending[i].expect {
        Expect::Button { labels } => button_index(labels, button)
            .ok_or_else(|| format!("dialog #{id} has no button {button:?}: its buttons are {labels:?}"))?,
        Expect::Color => unreachable!(),
        Expect::Paths { .. } => return Err(format!("dialog #{id} is a file dialog: answer it with `answer` and paths")),
    };
    let label = match &pending[i].expect {
        Expect::Button { labels } => labels[index].clone(),
        Expect::Color | Expect::Paths { .. } => unreachable!(),
    };
    let p = pending.remove(i);
    drop(pending);
    p.tx.send(Answer::Button(index)).map_err(|_| format!("dialog #{id} is gone"))?;
    crate::log_later("dialog", format!("[dialog] #{id} answered \"{label}\""));
    Ok(json!({ "id": id, "chose": label }))
}

/// `dialog cancel`: dismiss dialog `id`, as Esc would.
pub fn cancel(id: u64) -> Result<Json, String> {
    let p = remove(id).ok_or_else(|| format!("no dialog #{id} is waiting"))?;
    let _ = p.tx.send(Answer::Cancel);
    crate::log_later("dialog", format!("[dialog] #{id} cancelled"));
    Ok(json!({ "id": id, "cancelled": true }))
}
