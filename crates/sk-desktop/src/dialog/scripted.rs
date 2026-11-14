//! `SK_DIALOGS=scripted`: a dialog that waits for the devtools instead of the user. It still runs on its
//! own thread, owned and serialised like a real one: it parks a [`Pending`] and waits for `dialog answer` /
//! `choose` / `cancel`, polling its owner every 25 ms. Answers are validated as the OS dialog's flags
//! would; a rejected answer leaves the dialog waiting.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value as Json};

use super::ask::{label_of, Ask};
use super::{describe, Cancellation, Kind, Native};
use crate::Fail;

pub enum Answer {
    Paths(Vec<PathBuf>),
    Button(usize),
    Color(String),
    Cancel,
}

enum Expect {
    Color,
    Paths { kind: Kind, multiple: bool },
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

fn wait(window: u64, what: &'static str, expect: Expect, summary: Json, waiting: String, cancel: &Cancellation) -> Option<Answer> {
    let (tx, rx) = channel();
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    crate::log_later("dialog", format!("[dialog] #{id} {waiting} is waiting for a scripted answer (SK_DIALOGS=scripted)"));
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

pub(super) fn pick(n: &Native, window: u64, what: &'static str, cancel: &Cancellation) -> Result<Option<Vec<PathBuf>>, Fail> {
    let expect = Expect::Paths { kind: n.kind, multiple: n.multiple };
    let waiting = format!("{what} ({})", n.kind.as_str());
    Ok(match wait(window, what, expect, describe(n), waiting, cancel) {
        Some(Answer::Paths(p)) => Some(p),
        _ => None,
    })
}

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

fn button_index(labels: &[String], button: &str) -> Option<usize> {
    let b = button.trim();
    if let Ok(i) = b.parse::<usize>() {
        return (i < labels.len()).then_some(i);
    }
    let b = label_of(b).to_lowercase();
    labels.iter().position(|l| l.to_lowercase() == b)
}

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

pub fn cancel(id: u64) -> Result<Json, String> {
    let p = remove(id).ok_or_else(|| format!("no dialog #{id} is waiting"))?;
    let _ = p.tx.send(Answer::Cancel);
    crate::log_later("dialog", format!("[dialog] #{id} cancelled"));
    Ok(json!({ "id": id, "cancelled": true }))
}
