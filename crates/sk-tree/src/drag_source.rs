//! Browser-style source sessions for real native gestures. The JS data store is
//! rooted until the OS reports its result, including when the window closes.
mod download;
use crate::{render, EventPayload};
use gpui::{App, DropOperation, PlatformDragData, PlatformDragImage, Window};
use sk_runtime::{
    host_fn,
    redjs::{
        api::Value,
        webapi::{uievents, util::*},
    },
    JsRuntime,
};
use std::{
    cell::RefCell,
    ffi::c_void,
    hash::{BuildHasher, RandomState},
    sync::Arc,
    time::Duration,
};

pub(super) const SESSION_FORMAT: &str = "application/x-skathi-drag-session";
struct Source {
    token: String,
    node: u32,
    store: u64,
    realm: usize,
}
thread_local! { static ACTIVE: RefCell<Option<Source>> = const {RefCell::new(None)}; }

pub(super) unsafe fn target_store(
    g: *mut c_void,
    token: Option<&str>,
    mode: uievents::DragDataMode,
) -> Option<u64> {
    let store = ACTIVE.with_borrow(|s| {
        s.as_ref()
            .filter(|s| s.realm == g as usize && Some(s.token.as_str()) == token)
            .map(|s| s.store)
    })?;
    uievents::transfer_for_target(g, store, mode)
}

// Devtools supplies the compositor for synthetic mouse events, which have no
// OS button serial. DOM export, target negotiation and source completion all
// take their ordinary paths.
enum Driven {
    Idle,
    Armed,
    Drag(gpui::DragOffer, Box<dyn FnOnce(DropOperation)>),
}
thread_local! { static DRIVEN: RefCell<Driven> = const { RefCell::new(Driven::Idle) }; }

pub fn arm_document_drag() -> bool {
    DRIVEN.with_borrow_mut(|state| {
        if !matches!(state, Driven::Idle) {
            return false;
        }
        *state = Driven::Armed;
        true
    })
}
pub fn document_drag_offer() -> Option<gpui::DragOffer> {
    DRIVEN.with_borrow(|state| match state {
        Driven::Drag(offer, _) => Some(offer.clone()),
        _ => None,
    })
}
pub fn finish_document_drag(op: DropOperation) -> bool {
    match DRIVEN.replace(Driven::Idle) {
        Driven::Drag(_, done) => {
            done(op);
            true
        }
        Driven::Armed => true,
        Driven::Idle => false,
    }
}

fn get_source(token: &str) -> Option<(u32, u64)> {
    ACTIVE.with_borrow(|s| {
        s.as_ref()
            .filter(|s| s.token == token)
            .map(|s| (s.node, s.store))
    })
}
fn release(token: &str, restore: bool) {
    let source = ACTIVE.with_borrow_mut(|s| {
        if s.as_ref().is_some_and(|s| s.token == token) {
            s.take()
        } else {
            None
        }
    });
    if let Some(s) = source {
        JsRuntime::try_with(|rt| {
            if rt.raw() as usize == s.realm {
                unsafe {
                    uievents::set_drag_data_mode(
                        rt.raw(),
                        s.store,
                        if restore {
                            uievents::DragDataMode::ReadWrite
                        } else {
                            uievents::DragDataMode::Disabled
                        },
                    );
                    rt.ctx().unprotect(Value(s.store));
                }
            }
        });
    }
}

fn source_event(token: &str, kind: &str, op: DropOperation, w: &mut Window, cx: &mut App) {
    let Some((node, store)) = get_source(token) else {
        return;
    };
    let pos = w.mouse_position();
    let screen = w.bounds().origin + pos;
    let effects = ["none", "copy", "move", "", "link"];
    render::js_wave_app(w, cx, || {
        JsRuntime::with(|rt| {
            let _ = rt.ctx().set(
                Value(store),
                &rt.ctx().atom("dropEffect"),
                rt.ctx().string(effects[op as usize]),
            );
        });
        crate::pointer_at_next_dispatch(f32::from(screen.x) as f64, f32::from(screen.y) as f64);
        crate::dispatch_event_with(
            node,
            65,
            &EventPayload {
                a: f32::from(pos.x) as f64,
                b: f32::from(pos.y) as f64,
                key: Some(kind),
                ..Default::default()
            },
            Some(Value(store)),
        )
    });
}

fn preview(w: &Window, node: u32, hotspot: Option<(i32, i32)>) -> Option<Arc<PlatformDragImage>> {
    let (x, y, width, height) = render::painted_bounds(node)?;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let frame = w.render_to_image().ok()?;
    let scale = w.scale_factor();
    let left = (x * scale).floor().max(0.) as u32;
    let top = (y * scale).floor().max(0.) as u32;
    let right = (((x + width) * scale).ceil().max(0.) as u32).min(frame.width());
    let bottom = (((y + height) * scale).ceil().max(0.) as u32).min(frame.height());
    if right <= left || bottom <= top {
        return None;
    }
    let original =
        image::imageops::crop_imm(&frame, left, top, right - left, bottom - top).to_image();
    let mut pixels = if original.width().max(original.height()) > 512 {
        image::imageops::thumbnail(&original, 512, 512)
    } else {
        original
    };
    if hotspot.is_none() {
        for pixel in pixels.pixels_mut() {
            pixel[3] = (u16::from(pixel[3]) * 3 / 4) as u8;
        }
    }
    let pos = w.mouse_position();
    let (dx, dy) =
        hotspot.unwrap_or(((f32::from(pos.x) - x) as i32, (f32::from(pos.y) - y) as i32));
    let pixel_x = (((x + dx as f32) * scale - left as f32)
        * (pixels.width() as f32 / (right - left) as f32))
        .round() as i32;
    let pixel_y = (((y + dy as f32) * scale - top as f32)
        * (pixels.height() as f32 / (bottom - top) as f32))
        .round() as i32;
    Some(Arc::new(PlatformDragImage {
        width: pixels.width(),
        height: pixels.height(),
        hotspot: (
            pixel_x.clamp(0, pixels.width() as i32 - 1),
            pixel_y.clamp(0, pixels.height() as i32 - 1),
        ),
        rgba: pixels.into_raw(),
    }))
}

host_fn!(pub(super) fn sk_document_drag(g, args) {
    enc_bool(begin(g, crate::arg_id(args.arg(0)), args.arg(1).0))
});

fn begin(g: *mut c_void, node: u32, store: u64) -> bool {
    sk_runtime::with_commit(|window, cx| {
        let driven = DRIVEN.with_borrow(|state| matches!(state, Driven::Armed));
        if (!driven && !window.can_start_document_drag()) || ACTIVE.with_borrow(Option::is_some) {
            return false;
        }
        let Some(export) = (unsafe { uievents::export_drag(g, store) }) else {
            return false;
        };
        let allowed = match export.effects.as_str() {
            "none" => 0,
            "copy" => 1,
            "move" => 2,
            "link" => 4,
            "copyMove" => 3,
            "copyLink" => 5,
            "linkMove" => 6,
            _ => 7,
        };
        if allowed == 0 {
            return false;
        }
        let mut files: Vec<_> = export
            .files
            .into_iter()
            .map(|file| download::range_file(file.name, file.range, file.path, file.directory))
            .collect();
        if let Some((_, value)) = export
            .strings
            .iter()
            .find(|(mime, _)| mime == "downloadurl")
        {
            if let Some(file) = download::promised_download(g, value) {
                files.push(file);
            }
        }
        let token = format!(
            "{:016x}{:016x}",
            RandomState::new().hash_one(node),
            RandomState::new().hash_one(store)
        );
        let mut strings = export.strings;
        strings.retain(|(mime, _)| {
            matches!(mime.as_str(), "text/plain" | "text/html" | "text/uri-list")
        });
        strings.push((SESSION_FORMAT.into(), token.clone()));
        let custom = export.image.and_then(|(el, x, y)| {
            let id = unsafe { read_number(g, get(g, el, "__skId")) } as u32;
            (id != 0).then_some((id, Some((x, y))))
        });
        let image = custom
            .and_then(|(id, offset)| preview(window, id, offset))
            .or_else(|| preview(window, node, None));
        let data = PlatformDragData {
            strings,
            files,
            allowed,
            image,
        };
        JsRuntime::with(|rt| rt.ctx().protect(Value(store)));
        unsafe {
            uievents::set_drag_data_mode(g, store, uievents::DragDataMode::Protected);
        }
        ACTIVE.with_borrow_mut(|slot| {
            *slot = Some(Source {
                token: token.clone(),
                node,
                store,
                realm: g as usize,
            })
        });
        let win = window.window_handle();
        let executor = cx.foreground_executor().clone();
        let mut app = cx.to_async();
        let end = token.clone();
        let done: Box<dyn FnOnce(DropOperation)> = Box::new(move |op| {
            executor
                .spawn(async move {
                    let _ = win.update(&mut app, |_, win, cx| {
                        source_event(&end, "dragend", op, win, cx)
                    });
                    release(&end, false);
                })
                .detach();
        });
        let started = if driven {
            DRIVEN.set(Driven::Drag(
                gpui::DragOffer {
                    allowed: data.allowed,
                    strings: data.strings,
                    ..Default::default()
                },
                done,
            ));
            true
        } else {
            window.start_document_drag(data, done)
        };
        if !started {
            release(&token, true);
            return false;
        }
        let win = window.window_handle();
        cx.spawn(async move |cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            if get_source(&token).is_none() {
                break;
            }
            if win
                .update(cx, |_, w, cx| {
                    source_event(&token, "drag", DropOperation::None, w, cx)
                })
                .is_err()
            {
                release(&token, false);
                break;
            }
        })
        .detach();
        true
    })
    .unwrap_or(false)
}
