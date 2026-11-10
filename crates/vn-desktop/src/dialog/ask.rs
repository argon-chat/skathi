//! A question with buttons, asked as a modal of a window: the permission
//! prompt (crates/vn-permissions `consent`, docs/PERMISSIONS.md §5.1).
//!
//! It follows the file dialogs' threading and owner rules (mod.rs):
//!
//! - It is owned by the window published for the call that asks.
//! - It waits behind a dialog that window already has open, as a download
//!   does, rather than being refused.
//! - Its modal loop runs on a `vn-ask` thread of its own, so the frame thread
//!   keeps pumping.
//! - It closes when its owner starts to close. vn-app destroys that window
//!   only once the loop has exited (`owner_closing` / `is_idle`).
//! - Under `VN_DIALOGS=scripted` it never reaches the OS. It waits for the
//!   devtools `dialog choose` (a button) or `dialog cancel` (Esc).
//!
//! On Windows it is a TaskDialog (win.rs). TaskDialog makes it modal to its
//! owner only, gives it the keyboard (Tab, the arrow keys, Enter, the `&`
//! access keys) and makes Esc and the close box a dismissal.
//!
//! A caller may show the question itself instead ([`ask_with`]): vn-app's
//! permission prompt is a window of its own on Windows. Everything above
//! still holds for it (the owner, the queue, the close, the scripted seam);
//! only the native dialog is replaced, and a presenter that fails falls back
//! to it.

use super::*;

/// What to ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// What is asking, for the log and `dialog list`: `"permission"`.
    pub what: &'static str,
    /// The window title: the app's name.
    pub title: String,
    /// The question, in the dialog's large type.
    pub instruction: String,
    /// A sentence or two under it.
    pub content: String,
    /// The buttons, in order. `&` marks the access key.
    pub buttons: Vec<String>,
    /// The button Enter presses.
    pub default: usize,
}

/// Called exactly once: with the index of the button pressed, or `None` for a
/// dismissal (Esc, the close box, the owner window closing, or nobody able to
/// answer).
pub type Reply = Box<dyn FnOnce(Option<usize>) + Send>;

/// Asks `q` as a modal of the window published for this call (the primary
/// one outside a call). Never blocks: `reply` runs later, on the dialog's
/// thread.
pub fn ask(q: Ask, reply: Reply) {
    ask_owned(RESOLVER.get().and_then(|f| f()), q, None, reply)
}

/// Shows a question in place of the platform's dialog. It runs on the
/// `vn-ask` thread once the question's turn has come, with the owner's native
/// window (0 for none), and returns when the question has gone away: with the
/// button, or `None` for a dismissal. While it runs it watches the
/// [`Cancellation`]: its owner is closing, and is destroyed only after it has
/// returned. An `Err` means it could not show the question at all, and the
/// platform's dialog asks instead.
pub type Present = Box<dyn FnOnce(&Ask, isize, &Cancellation) -> Result<Option<usize>, Fail> + Send>;

/// [`ask`], shown by `present` rather than by the platform's dialog. Under
/// `VN_DIALOGS=scripted` `present` is never called: the devtools answer, as
/// they do for [`ask`].
pub fn ask_with(q: Ask, present: Present, reply: Reply) {
    ask_owned(RESOLVER.get().and_then(|f| f()), q, Some(present), reply)
}

/// The reply, sent once: with the answer, or with `None` when whatever holds
/// it goes away first (a dialog thread that could not start, or panicked).
struct Once(Option<Reply>);

impl Once {
    fn send(mut self, v: Option<usize>) {
        if let Some(f) = self.0.take() {
            f(v);
        }
    }
}

impl Drop for Once {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f(None);
        }
    }
}

/// A button's label as the user reads it: `&Allow` is "Allow".
pub fn label_of(button: &str) -> String {
    let mut out = String::with_capacity(button.len());
    let mut chars = button.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            // `&&` is a literal ampersand.
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    out
}

pub(super) fn ask_owned(owner: Option<Owner>, q: Ask, present: Option<Present>, reply: Reply) {
    let key = owner.map(|o| o.key).unwrap_or(0);
    let hwnd = owner.map(|o| o.hwnd).unwrap_or(0);
    let st = owner_state(key);
    let reply = Once(Some(reply));
    if st.closed.load(Ordering::Acquire) || q.buttons.is_empty() {
        reply.send(None);
        return;
    }
    // Queued here, on the frame thread, so the order is the order asked.
    let ticket = st.ticket.fetch_add(1, Ordering::AcqRel);
    st.queue.lock().unwrap_or_else(|p| p.into_inner()).push_back(ticket);
    let cancel = Cancellation(st.clone());
    let spawned = std::thread::Builder::new().name("vn-ask".into()).spawn(move || {
        if !wait_turn(&st, ticket, &cancel) {
            reply.send(None);
            return;
        }
        let answer = if scripted_mode() {
            scripted::ask(&q, key, &cancel)
        } else {
            let shown = match present {
                Some(present) => present(&q, hwnd, &cancel).or_else(|f| {
                    crate::log_later(
                        "dialog",
                        format!("[dialog] the {} window could not be shown ({}): asking natively", q.what, f.message),
                    );
                    imp::ask(&q, hwnd, &cancel)
                }),
                None => imp::ask(&q, hwnd, &cancel),
            };
            match shown {
                Ok(a) => a,
                Err(f) => {
                    crate::log_later("dialog", format!("[dialog] the {} dialog could not be shown: {}", q.what, f.message));
                    None
                }
            }
        };
        // The native loop has exited: the owner may be destroyed now.
        st.active.store(false, Ordering::Release);
        let answer = answer.filter(|&i| i < q.buttons.len() && !cancel.is_requested());
        let said = match answer {
            Some(i) => format!("\"{}\"", label_of(&q.buttons[i])),
            None => "dismissed".to_string(),
        };
        crate::log_later("dialog", format!("[dialog] {} \"{}\": {said}", q.what, q.instruction));
        reply.send(answer);
    });
    if spawned.is_err() {
        // The closure went down with the failed spawn, and its reply said
        // `None`. Only the place in the queue is left to give back.
        owner_state(key).leave_queue(ticket);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    static KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 41);

    fn owner() -> Option<Owner> {
        Some(Owner { key: KEY.fetch_add(1, Ordering::Relaxed), hwnd: 0 })
    }

    fn question() -> Ask {
        Ask {
            what: "permission",
            title: "Test".into(),
            instruction: "Test wants to use your microphone".into(),
            content: "Choose.".into(),
            buttons: vec!["&Allow".into(), "&Block".into()],
            default: 0,
        }
    }

    fn pending_of(window: u64) -> u64 {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(v) = scripted::list().as_array().unwrap().iter().find(|v| v["window"] == window) {
                return v["id"].as_u64().unwrap();
            }
            assert!(std::time::Instant::now() < deadline, "no scripted question appeared for window {window}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn asked(o: Option<Owner>) -> std::sync::mpsc::Receiver<Option<usize>> {
        let (tx, rx) = channel();
        ask_owned(o, question(), None, Box::new(move |a| tx.send(a).unwrap()));
        rx
    }

    #[test]
    fn a_presented_question_is_still_answered_by_the_script() {
        let o = owner();
        let key = o.unwrap().key;
        let (tx, rx) = channel();
        let present: Present = Box::new(|_, _, _| panic!("VN_DIALOGS=scripted must never show a window"));
        ask_owned(o, question(), Some(present), Box::new(move |a| tx.send(a).unwrap()));
        scripted::choose(pending_of(key), "allow").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), Some(0));
        assert!(is_idle(key));
    }

    #[test]
    fn a_label_drops_its_access_key_marker() {
        assert_eq!(label_of("&Allow"), "Allow");
        assert_eq!(label_of("Save && &Quit"), "Save & Quit");
        assert_eq!(label_of("&Разрешить"), "Разрешить");
    }

    #[test]
    fn a_scripted_question_is_listed_and_answered_by_button_or_dismissed() {
        let o = owner();
        let key = o.unwrap().key;
        let rx = asked(o);
        let id = pending_of(key);
        let listed = scripted::list();
        let v = listed.as_array().unwrap().iter().find(|v| v["id"] == id).unwrap().clone();
        assert_eq!(v["kind"], "ask");
        assert_eq!(v["what"], "permission");
        assert_eq!(v["buttons"], serde_json::json!(["Allow", "Block"]));
        assert_eq!(v["instruction"], "Test wants to use your microphone");
        // A question takes a button, not paths, and only one that exists.
        assert!(scripted::answer(id, vec![std::env::temp_dir()]).is_err());
        assert!(scripted::choose(id, "Maybe").is_err());
        assert!(scripted::choose(id, "2").is_err());
        scripted::choose(id, "block").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), Some(1));
        assert!(is_idle(key));
        // By index, and Esc.
        let rx = asked(o);
        scripted::choose(pending_of(key), "0").unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), Some(0));
        let rx = asked(o);
        scripted::cancel(pending_of(key)).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), None);
    }

    #[test]
    fn a_question_waits_behind_an_open_picker_and_closes_with_its_window() {
        let o = owner();
        let key = o.unwrap().key;
        let (n, then) = parse(r#"{"kind":"open","result":"handles"}"#, vec![]).unwrap();
        let picker = crate::tests::token_of(&start_owned(o, n, then, "picker"));
        let first = pending_of(key);
        let rx = asked(o);
        std::thread::sleep(QUEUE_POLL * 3);
        assert_eq!(
            scripted::list().as_array().unwrap().iter().filter(|v| v["window"] == key).count(),
            1,
            "only the picker is in front of the user"
        );
        scripted::cancel(first).unwrap();
        let _ = crate::tests::wait_for_settles(&[picker]);
        let id = pending_of(key);
        assert_ne!(id, first);
        // The window closes under the question: a dismissal.
        assert!(owner_closing(key));
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), None);
        // A window already closing answers at once, without asking.
        let rx = asked(o);
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), None);
    }
}
