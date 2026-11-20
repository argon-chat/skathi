//! Routes a platform drag to the element painted in the receiving window.
use gpui::{App, DragOffer, DropOperation, ExternalPaths, FileDropEvent, Pixels, Point, Window};
use sk_runtime::redjs::api::Value;
use sk_runtime::redjs::webapi::uievents::{external_drag_data, set_drag_data_mode, DragDataMode};
use sk_runtime::JsRuntime;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
const OVER_INTERVAL: Duration = Duration::from_millis(50);

use crate::{pointer, render, windows, EventPayload};

#[derive(Default)]
pub(crate) struct Session {
    paths: Vec<PathBuf>,
    strings: Vec<(String, String)>,
    source_token: Option<String>,
    over: u32,
    op: DropOperation,
    position: Point<Pixels>,
    id: u64,
    last_over: Option<Instant>,
    offer: DragOffer,
}

const EFFECTS: [&str; 8] = [
    "none", "copy", "move", "copyMove", "link", "copyLink", "linkMove", "all",
];

fn preferred(offer: &DragOffer) -> DropOperation {
    use DropOperation::*;
    let m = offer.modifiers;
    let first = if m.alt || (m.control && m.shift) {
        Link
    } else if m.shift {
        Move
    } else {
        Copy
    };
    [first, Copy, Move, Link]
        .into_iter()
        .find(|op| offer.allowed & *op as u8 != 0)
        .unwrap_or(None)
}

fn effect(op: DropOperation) -> &'static str {
    EFFECTS[op as usize]
}

impl Session {
    fn send(
        &self,
        event: &str,
        target: u32,
        related: u32,
        offer: &DragOffer,
        screen: Point<Pixels>,
    ) -> DropOperation {
        if target == 0 {
            return DropOperation::None;
        }
        let mode = if event == "drop" {
            DragDataMode::ReadOnly
        } else {
            DragDataMode::Protected
        };
        let wanted = if event == "drop" {
            self.op
        } else {
            preferred(offer)
        };
        let store = JsRuntime::with(|rt| unsafe {
            let dt = Value(
                crate::drag_source::target_store(rt.raw(), self.source_token.as_deref(), mode)
                    .unwrap_or_else(|| {
                        external_drag_data(
                            rt.raw(),
                            &self.paths,
                            &self.strings,
                            mode,
                            EFFECTS[(offer.allowed & 7) as usize],
                            |path| {
                                render::warn_compat(
                                    "drop:unreadable",
                                    &format!(
                            "Could not open dropped path '{}'; delivering the remaining files.",
                            path.display()
                        ),
                                );
                            },
                        )
                    }),
            );
            rt.ctx().protect(dt);
            let _ = rt.ctx().set(
                dt,
                &rt.ctx().atom("dropEffect"),
                rt.ctx().string(effect(wanted)),
            );
            dt
        });
        let p = EventPayload {
            a: f32::from(self.position.x) as f64,
            b: f32::from(self.position.y) as f64,
            c: related as f64,
            d: render::mods_bits(&offer.modifiers),
            key: Some(event),
        };
        crate::pointer_at_next_dispatch(f32::from(screen.x) as f64, f32::from(screen.y) as f64);
        let proceed = crate::dispatch_event_with(target, 64, &p, Some(store));
        JsRuntime::with(|rt| unsafe {
            let operation = rt
                .ctx()
                .get(store, &rt.ctx().atom("dropEffect"))
                .ok()
                .and_then(|v| rt.ctx().to_rust_string(v).ok());
            set_drag_data_mode(rt.raw(), store.0, DragDataMode::Disabled);
            rt.ctx().unprotect(store);
            if proceed {
                return DropOperation::None;
            }
            [
                DropOperation::Copy,
                DropOperation::Move,
                DropOperation::Link,
            ]
            .into_iter()
            .find(|op| operation.as_deref() == Some(effect(*op)) && offer.allowed & *op as u8 != 0)
            .unwrap_or(DropOperation::None)
        })
    }

    fn update(&mut self, target: u32, offer: &DragOffer, screen: Point<Pixels>) {
        if target != self.over {
            self.send("dragenter", target, self.over, offer, screen);
            self.send("dragleave", self.over, target, offer, screen);
            self.over = target;
        }
        self.op = self.send("dragover", target, 0, offer, screen);
    }
}

pub(crate) fn handle(
    event: &FileDropEvent,
    offer: &DragOffer,
    window: &mut Window,
    cx: &mut App,
) -> DropOperation {
    let mut session = windows::here(window, |w| w.file_drop.take());
    let position = match event {
        FileDropEvent::Entered { position, .. }
        | FileDropEvent::Pending { position }
        | FileDropEvent::Submit { position } => *position,
        FileDropEvent::Exited | FileDropEvent::Ended => {
            session.as_ref().map(|s| s.position).unwrap_or_default()
        }
    };
    let target = pointer::target_at(window, position);
    let screen = window.bounds().origin + position;
    let mut answer = DropOperation::None;
    render::js_wave_app(window, cx, || {
        if let FileDropEvent::Entered {
            paths: ExternalPaths(paths),
            ..
        } = event
        {
            if let Some(old) = session.take() {
                old.send("dragleave", old.over, 0, offer, screen);
            }
            session = Some(Session {
                paths: paths.to_vec(),
                strings: offer
                    .strings
                    .iter()
                    .filter(|(mime, _)| mime != crate::drag_source::SESSION_FORMAT)
                    .cloned()
                    .collect(),
                source_token: offer
                    .strings
                    .iter()
                    .find(|(mime, _)| mime == crate::drag_source::SESSION_FORMAT)
                    .map(|(_, token)| token.clone()),
                id: NEXT_SESSION.fetch_add(1, Ordering::Relaxed),
                ..Default::default()
            });
        }
        let Some(s) = session.as_mut() else {
            return true;
        };
        s.position = position;
        match event {
            FileDropEvent::Entered { .. } | FileDropEvent::Pending { .. } => {
                s.update(target, offer, screen);
                s.last_over = Some(Instant::now());
                s.offer.allowed = offer.allowed;
                s.offer.modifiers = offer.modifiers;
                answer = s.op;
            }
            FileDropEvent::Submit { .. } => {
                if target != s.over {
                    s.update(target, offer, screen);
                }
                if s.op != DropOperation::None {
                    answer = s.send("drop", target, 0, offer, screen);
                } else {
                    s.send("dragleave", s.over, 0, offer, screen);
                }
                session = None;
            }
            FileDropEvent::Exited | FileDropEvent::Ended => {
                s.send("dragleave", s.over, 0, offer, screen);
                session = None;
            }
        }
        true
    });
    if !cfg!(target_os = "windows") && matches!(event, FileDropEvent::Entered { .. }) {
        if let Some(s) = &session {
            repeat_over(s.id, window, cx);
        }
    }
    windows::here(window, |w| w.file_drop = session);
    answer
}

fn repeat_over(id: u64, window: &Window, cx: &App) {
    let window_handle = window.window_handle();
    cx.spawn(async move |cx| loop {
        cx.background_executor().timer(OVER_INTERVAL).await;
        let live = window_handle
            .update(cx, |_, window, cx| {
                let current = windows::here(window, |w| {
                    w.file_drop
                        .as_ref()
                        .filter(|s| s.id == id)
                        .map(|s| (s.position, s.offer.clone(), s.last_over))
                });
                let Some((position, mut offer, last)) = current else {
                    return false;
                };
                if last.is_some_and(|t| t.elapsed() >= OVER_INTERVAL) {
                    offer.modifiers = window.modifiers();
                    handle(&FileDropEvent::Pending { position }, &offer, window, cx);
                }
                true
            })
            .unwrap_or(false);
        if !live {
            break;
        }
    })
    .detach();
}
