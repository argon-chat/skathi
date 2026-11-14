//! `MediaMetadata` and `navigator.mediaSession` for the system media controls. Changes in one task reach the
//! system as one update; artwork the system does not load itself is read with the page's `fetch` and sent
//! as bytes.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;

use serde_json::json;
use sk_runtime::host_fn;
use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use super::global;

const ACTIONS: [&str; 17] = [
    "play",
    "pause",
    "seekbackward",
    "seekforward",
    "previoustrack",
    "nexttrack",
    "skipad",
    "stop",
    "seekto",
    "togglemicrophone",
    "togglecamera",
    "togglescreenshare",
    "hangup",
    "previousslide",
    "nextslide",
    "enterpictureinpicture",
    "voiceactivity",
];

#[derive(Clone)]
struct Image {
    src: String,
    sizes: String,
}

#[derive(Default)]
struct Metadata {
    title: String,
    artist: String,
    album: String,
    artwork: Vec<Image>,
}

#[derive(Default)]
struct Session {
    metadata: u64,
    playback: &'static str,
    handlers: Vec<&'static str>,
    position: Option<(f64, f64, f64)>,
    queued: bool,
    generation: u32,
    art: Option<(String, Vec<u8>)>,
}

thread_local! {
    static CLASS: Cell<usize> = const { Cell::new(0) };
    static PROTO: Cell<u64> = const { Cell::new(0) };
    static SESSION: Cell<u64> = const { Cell::new(0) };
    static BAG: Cell<u64> = const { Cell::new(0) };
    static ARTWORK: Cell<usize> = const { Cell::new(0) };
    static STATE: RefCell<Session> = RefCell::new(Session { playback: "none", ..Default::default() });
}

fn class() -> JSClassRef {
    CLASS.with(|c| c.get()) as JSClassRef
}

unsafe fn metadata<'a>(g: *mut c_void, v: u64) -> Option<&'a mut Metadata> {
    host_data::<Metadata>(g, class(), v)
}

unsafe fn string_member(g: *mut c_void, obj: u64, key: &str) -> Result<String, ()> {
    if !is_cell(obj) {
        return Ok(String::new());
    }
    let v = try_get(g, obj, key)?;
    if v == UNDEF {
        return Ok(String::new());
    }
    Ok(read_string(g, to_js_string(g, v)?).unwrap_or_default())
}

unsafe fn resolve_url(g: *mut c_void, src: &str) -> Option<String> {
    let reflect = get(g, global(g), "Reflect");
    let args = new_array(g);
    set_i(g, args, 0, js_string(g, src));
    let location = get(g, global(g), "location");
    if is_cell(location) {
        set_i(g, args, 1, get(g, location, "href"));
    }
    let url = call(
        g,
        get(g, reflect, "construct"),
        reflect,
        &[get(g, global(g), "URL"), args],
    )
    .ok()?;
    read_string(g, get(g, url, "href"))
}

/// The artwork list, or a TypeError thrown.
unsafe fn images(g: *mut c_void, list: u64) -> Result<Vec<Image>, ()> {
    if list == UNDEF {
        return Ok(Vec::new());
    }
    let from = get(g, get(g, global(g), "Array"), "from");
    let Ok(arr) = call(g, from, UNDEF, &[list]) else {
        return Err(());
    };
    let mut out = Vec::new();
    for i in 0..array_len(g, arr) {
        let img = get_i(g, arr, i);
        let src = if is_cell(img) {
            try_get(g, img, "src")?
        } else {
            UNDEF
        };
        if src == UNDEF {
            throw_type_error(g, "Failed to read the 'src' property from 'MediaImage': Required member is undefined.");
            return Err(());
        }
        let raw = read_string(g, to_js_string(g, src)?).unwrap_or_default();
        let Some(src) = resolve_url(g, &raw) else {
            throw_type_error(g, &format!("Failed to construct 'MediaMetadata': '{raw}' can't be resolved to a valid URL."));
            return Err(());
        };
        out.push(Image {
            src,
            sizes: string_member(g, img, "sizes")?,
        });
        let _ = string_member(g, img, "type")?;
    }
    Ok(out)
}

/// The frozen array `artwork` returns, made once per assignment.
unsafe fn store_artwork(g: *mut c_void, obj: u64, artwork: &[Image], types: &[String]) {
    let object = get(g, global(g), "Object");
    let freeze = get(g, object, "freeze");
    let arr = new_array(g);
    for (i, img) in artwork.iter().enumerate() {
        let o = new_object(g, UNDEF);
        define(g, o, "src", js_string(g, &img.src), 0);
        define(g, o, "sizes", js_string(g, &img.sizes), 0);
        define(
            g,
            o,
            "type",
            js_string(g, types.get(i).map_or("", String::as_str)),
            0,
        );
        let _ = call(g, freeze, object, &[o]);
        set_i(g, arr, i as u32, o);
    }
    let _ = call(g, freeze, object, &[arr]);
    glue::rjs_define(g, obj, ARTWORK.with(|a| a.get()) as *mut c_void, arr, 0);
}

unsafe fn types_of(g: *mut c_void, list: u64) -> Vec<String> {
    if list == UNDEF {
        return Vec::new();
    }
    let Ok(arr) = call(
        g,
        get(g, get(g, global(g), "Array"), "from"),
        UNDEF,
        &[list],
    ) else {
        return Vec::new();
    };
    (0..array_len(g, arr))
        .map(|i| string_member(g, get_i(g, arr, i), "type").unwrap_or_default())
        .collect()
}

host_fn!(fn metadata_construct(g, args) {
    unsafe {
        let init = args.arg(0).0;
        if init != UNDEF && init != NULL && !is_cell(init) {
            return throw_type_error(g, "Failed to construct 'MediaMetadata': The provided value is not of type 'MediaMetadataInit'.");
        }
        let (Ok(title), Ok(artist), Ok(album)) = (string_member(g, init, "title"), string_member(g, init, "artist"), string_member(g, init, "album")) else { return UNDEF };
        let list = if is_cell(init) { get(g, init, "artwork") } else { UNDEF };
        let Ok(artwork) = images(g, list) else { return UNDEF };
        let types = types_of(g, list);
        let obj = host_create(g, class(), PROTO.with(|p| p.get()), Metadata { title, artist, album, artwork: artwork.clone() });
        store_artwork(g, obj, &artwork, &types);
        obj
    }
});

macro_rules! text_field {
    ($get:ident, $set:ident, $field:ident) => {
        host_fn!(fn $get(g, args) {
            unsafe {
                match metadata(g, args.this().0) {
                    Some(m) => js_string(g, &m.$field),
                    None => throw_type_error(g, "Illegal invocation"),
                }
            }
        });
        host_fn!(fn $set(g, args) {
            unsafe {
                let this = args.this().0;
                let Ok(s) = to_js_string(g, args.arg(0).0) else { return UNDEF };
                let text = read_string(g, s).unwrap_or_default();
                match metadata(g, this) {
                    Some(m) => m.$field = text,
                    None => return throw_type_error(g, "Illegal invocation"),
                }
                changed_if_current(g, this);
            }
            UNDEF
        });
    };
}

text_field!(get_title, set_title, title);
text_field!(get_artist, set_artist, artist);
text_field!(get_album, set_album, album);

host_fn!(fn get_artwork(g, args) {
    unsafe {
        let this = args.this().0;
        if metadata(g, this).is_none() {
            return throw_type_error(g, "Illegal invocation");
        }
        get_a(g, this, ARTWORK.with(|a| a.get()) as *mut c_void)
    }
});

host_fn!(fn set_artwork(g, args) {
    unsafe {
        let this = args.this().0;
        if metadata(g, this).is_none() {
            return throw_type_error(g, "Illegal invocation");
        }
        let list = args.arg(0).0;
        let Ok(artwork) = images(g, list) else { return UNDEF };
        store_artwork(g, this, &artwork, &types_of(g, list));
        if let Some(m) = metadata(g, this) {
            m.artwork = artwork;
        }
        changed_if_current(g, this);
    }
    UNDEF
});

unsafe fn changed_if_current(g: *mut c_void, obj: u64) {
    if STATE.with(|s| s.borrow().metadata) == obj {
        changed(g);
    }
}

unsafe fn changed(g: *mut c_void) {
    if STATE.with(|s| std::mem::replace(&mut s.borrow_mut().queued, true)) {
        return;
    }
    let qm = get(g, global(g), "queueMicrotask");
    let _ = call(g, qm, UNDEF, &[make_fn(g, "", 0, flush_now)]);
}

fn area(sizes: &str) -> f64 {
    let mut best = 0.0;
    for s in sizes.to_ascii_lowercase().split_whitespace() {
        if s == "any" {
            return f64::INFINITY;
        }
        if let Some((w, h)) = s.split_once('x') {
            if let (Ok(w), Ok(h)) = (w.parse::<f64>(), h.parse::<f64>()) {
                best = f64::max(best, w * h);
            }
        }
    }
    best
}

fn send(state: &serde_json::Value, bytes: Option<&[u8]>) {
    if let Err(e) = crate::media_session::update(crate::dialog::current_owner().map(|o| o.hwnd), state, bytes) {
        crate::advise(&format!(
            "navigator.mediaSession: the system media controls refused the update ({e})"
        ));
    }
    #[cfg(test)]
    tests::SENT.with(|s| s.borrow_mut().push((state.clone(), bytes.map(<[u8]>::len))));
}

host_fn!(fn flush_now(g, _args) {
    unsafe { flush(g) };
    UNDEF
});

unsafe fn flush(g: *mut c_void) {
    let (meta_obj, playback, actions, position, generation) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.queued = false;
        s.generation += 1;
        (
            s.metadata,
            s.playback,
            s.handlers.clone(),
            s.position,
            s.generation,
        )
    });
    let meta = metadata(g, meta_obj);
    let art = meta.as_ref().and_then(|m| {
        m.artwork.iter().fold(None::<&Image>, |best, i| match best {
            Some(b) if area(&i.sizes) <= area(&b.sizes) => Some(b),
            _ => Some(i),
        })
    });
    let web = crate::media_session::LOADS_ARTWORK_URLS
        && art.is_some_and(|a| a.src.starts_with("http://") || a.src.starts_with("https://"));
    let mut state = json!({
        "metadata": meta.as_ref().map(|m| {
            let mut v = json!({ "title": m.title, "artist": m.artist, "album": m.album });
            if web {
                v["artworkUrl"] = art.map(|a| a.src.clone()).into();
            }
            v
        }),
        "playbackState": playback,
        "actions": actions,
        "position": position.map(|(d, p, r)| json!({ "duration": if d.is_finite() { d } else { 0.0 }, "position": p, "playbackRate": r })),
    });
    let Some(art) = art.filter(|_| !web).cloned() else {
        STATE.with(|s| s.borrow_mut().art = None);
        return send(&state, None);
    };
    let cached = STATE.with(|s| {
        s.borrow()
            .art
            .as_ref()
            .filter(|(src, _)| *src == art.src)
            .map(|(_, b)| b.clone())
    });
    if let Some(bytes) = cached {
        return send(&state, Some(&bytes));
    }
    send(&state, None);
    state["artSrc"] = art.src.clone().into();
    PENDING_STATE.with(|p| *p.borrow_mut() = Some((generation, state)));
    fetch_artwork(g, &art.src, generation);
}

thread_local! {
    static PENDING_STATE: RefCell<Option<(u32, serde_json::Value)>> = const { RefCell::new(None) };
}

unsafe fn fetch_artwork(g: *mut c_void, src: &str, generation: u32) {
    let fetch = get(g, global(g), "fetch");
    let Ok(p) = call(g, fetch, UNDEF, &[js_string(g, src)]) else {
        return;
    };
    let Ok(p) = call(g, get(g, p, "then"), p, &[make_fn(g, "", 1, read_body)]) else {
        return;
    };
    let done = make_fn(g, "", 2, artwork_read);
    let Ok(bound) = call(
        g,
        get(g, done, "bind"),
        done,
        &[UNDEF, enc_f64(generation as f64)],
    ) else {
        return;
    };
    let _ = call(g, get(g, p, "then"), p, &[bound, make_fn(g, "", 0, ignore)]);
}

host_fn!(fn read_body(g, args) {
    unsafe {
        let r = args.arg(0).0;
        call(g, get(g, r, "arrayBuffer"), r, &[]).unwrap_or(UNDEF)
    }
});

host_fn!(fn ignore(_g, _args) {
    UNDEF
});

host_fn!(fn artwork_read(g, args) {
    unsafe {
        let generation = args.arg(0).as_number().unwrap_or(-1.0) as u32;
        let Some(bytes) = bufferish(g, args.arg(1).0).map(<[u8]>::to_vec) else { return UNDEF };
        let Some((gen, mut state)) = PENDING_STATE.with(|p| p.borrow_mut().take()) else { return UNDEF };
        if gen != generation || STATE.with(|s| s.borrow().generation) != generation {
            return UNDEF;
        }
        let src = state.as_object_mut().and_then(|o| o.remove("artSrc")).and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        send(&state, Some(&bytes));
        STATE.with(|s| s.borrow_mut().art = Some((src, bytes)));
    }
    UNDEF
});

host_fn!(fn get_metadata(_g, _args) {
    STATE.with(|s| match s.borrow().metadata {
        0 => NULL,
        m => m,
    })
});

host_fn!(fn set_metadata(g, args) {
    unsafe {
        let v = args.arg(0).0;
        if v != NULL && v != UNDEF && metadata(g, v).is_none() {
            return throw_type_error(g, "Failed to set the 'metadata' property on 'MediaSession': The provided value is not of type 'MediaMetadata'.");
        }
        let next = if v == NULL || v == UNDEF { 0 } else { v };
        let prev = STATE.with(|s| std::mem::replace(&mut s.borrow_mut().metadata, next));
        if prev != 0 {
            redjs_unprotect(g, prev);
        }
        if next != 0 {
            redjs_protect(g, next);
        }
        changed(g);
    }
    UNDEF
});

host_fn!(fn get_playback_state(g, _args) {
    unsafe { js_string(g, STATE.with(|s| s.borrow().playback)) }
});

host_fn!(fn set_playback_state(g, args) {
    unsafe {
        let Ok(s) = to_js_string(g, args.arg(0).0) else { return UNDEF };
        let v = read_string(g, s).unwrap_or_default();
        let Some(state) = ["none", "paused", "playing"].into_iter().find(|x| *x == v) else { return UNDEF };
        STATE.with(|s| s.borrow_mut().playback = state);
        changed(g);
    }
    UNDEF
});

host_fn!(fn set_action_handler(g, args) {
    unsafe {
        let Ok(s) = to_js_string(g, args.arg(0).0) else { return UNDEF };
        let name = read_string(g, s).unwrap_or_default();
        let Some(action) = ACTIONS.into_iter().find(|a| *a == name) else {
            return throw_type_error(g, &format!("Failed to execute 'setActionHandler' on 'MediaSession': The provided value '{name}' is not a valid enum value of type MediaSessionAction."));
        };
        let handler = args.arg(1).0;
        let callable = sk_runtime::redjs::webapi::events::is_callable(g, handler);
        if handler != NULL && !callable {
            return throw_type_error(g, "Failed to execute 'setActionHandler' on 'MediaSession': The callback provided as parameter 2 is not a function.");
        }
        define(g, BAG.with(|b| b.get()), action, if callable { handler } else { NULL }, 0);
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.handlers.retain(|a| *a != action);
            if callable {
                s.handlers.push(action);
            }
        });
        changed(g);
    }
    UNDEF
});

host_fn!(fn set_position_state(g, args) {
    unsafe {
        let fail = |m: &str| throw_type_error(g, &format!("Failed to execute 'setPositionState' on 'MediaSession': {m}"));
        let st = args.arg(0).0;
        let member = |k: &str| if is_cell(st) { get(g, st, k) } else { UNDEF };
        let (d, p, r) = (member("duration"), member("position"), member("playbackRate"));
        if d == UNDEF && p == UNDEF && r == UNDEF {
            STATE.with(|s| s.borrow_mut().position = None);
            changed(g);
            return UNDEF;
        }
        if d == UNDEF {
            return fail("The duration must be provided.");
        }
        let Ok(duration) = to_number(g, d) else { return UNDEF };
        if duration.is_nan() || duration < 0.0 {
            return fail("The provided duration cannot be less than zero.");
        }
        let at = if p == UNDEF { 0.0 } else { let Ok(n) = to_number(g, p) else { return UNDEF }; n };
        if !at.is_finite() {
            return fail("The provided double value is non-finite.");
        }
        if at < 0.0 {
            return fail("The provided position cannot be less than zero.");
        }
        if at > duration {
            return fail("The provided position cannot be greater than the duration.");
        }
        let rate = if r == UNDEF { 1.0 } else { let Ok(n) = to_number(g, r) else { return UNDEF }; n };
        if !rate.is_finite() {
            return fail("The provided double value is non-finite.");
        }
        if rate == 0.0 {
            return fail("The provided playbackRate cannot be equal to zero.");
        }
        STATE.with(|s| s.borrow_mut().position = Some((duration, at, rate)));
        changed(g);
    }
    UNDEF
});

/// A system button or seek: the page's handler for it, if any.
pub(super) unsafe fn on_action(g: *mut c_void, payload: &serde_json::Value) {
    let action = payload["action"].as_str().unwrap_or("");
    let handler = get(g, BAG.with(|b| b.get()), action);
    if !is_cell(handler) {
        return;
    }
    let details = new_object(g, UNDEF);
    define(g, details, "action", js_string(g, action), 0);
    if let Some(t) = payload["seekTime"].as_f64() {
        define(g, details, "seekTime", enc_f64(t), 0);
    }
    if let Err(e) = call(g, handler, SESSION.with(|s| s.get()), &[details]) {
        let report = get(g, global(g), "reportError");
        let _ = call(g, report, UNDEF, &[e]);
    }
}

host_fn!(fn navigator_media_session(_g, _args) {
    SESSION.with(|s| s.get())
});

pub(super) unsafe fn install(g: *mut c_void, nav: u64) {
    CLASS.with(|c| c.set(make_host_class::<Metadata>("MediaMetadata") as usize));
    ARTWORK.with(|a| a.set(atom_priv(g, "sk#artwork") as usize));
    let meta = define_class(
        g,
        &ClassSpec {
            name: "MediaMetadata",
            arity: 0,
            construct: Some(metadata_construct),
            parent_proto: 0,
            methods: &[],
            getters: &[],
            accessors: &[
                ("title", get_title, set_title),
                ("artist", get_artist, set_artist),
                ("album", get_album, set_album),
                ("artwork", get_artwork, set_artwork),
            ],
        },
    );
    PROTO.with(|p| p.set(meta.proto));
    let session = define_class(
        g,
        &ClassSpec {
            name: "MediaSession",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[
                ("setActionHandler", 2, set_action_handler),
                ("setPositionState", 0, set_position_state),
            ],
            getters: &[],
            accessors: &[
                ("metadata", get_metadata, set_metadata),
                ("playbackState", get_playback_state, set_playback_state),
            ],
        },
    );
    let instance = new_object(g, session.proto);
    let bag = new_object(g, UNDEF);
    redjs_protect(g, instance);
    redjs_protect(g, bag);
    SESSION.with(|s| s.set(instance));
    BAG.with(|b| b.set(bag));
    super::define_getter(g, nav, "mediaSession", navigator_media_session);
}

#[cfg(test)]
pub(super) mod tests {
    use std::cell::RefCell;

    thread_local! {
        pub static SENT: RefCell<Vec<(serde_json::Value, Option<usize>)>> = const { RefCell::new(Vec::new()) };
    }

    fn sent() -> Vec<(serde_json::Value, Option<usize>)> {
        sk_runtime::testing::on_js_thread(|| SENT.with(|s| s.borrow_mut().drain(..).collect()))
    }

    #[test]
    fn one_update_per_task_and_buttons_reach_the_handlers() {
        let _ = sent();
        let out = super::super::testing::run(
            "const s = navigator.mediaSession; const log = []; \
             s.metadata = new MediaMetadata({ title: 'Song', artist: 'Band', artwork: [{ src: 'https://x.test/s.png', sizes: '96x96' }, { src: 'https://x.test/l.png', sizes: '512x512', type: 'image/png' }] }); \
             s.playbackState = 'playing'; s.playbackState = 'bogus'; \
             s.setActionHandler('play', () => {}); s.setActionHandler('seekto', (d) => { globalThis.seek = d.action + ':' + d.seekTime; }); s.setActionHandler('play', null); \
             s.setPositionState({ duration: 100, position: 5 }); \
             const art = s.metadata.artwork; log.push(art === s.metadata.artwork, Object.isFrozen(art), art[1].type); \
             for (const bad of [() => s.setActionHandler('fly', null), () => s.setPositionState({ duration: 10, position: 11 }), () => { s.metadata = {}; }, () => new MediaMetadata({ artwork: [{}] })]) { try { bad(); log.push('no'); } catch (e) { log.push(e instanceof TypeError); } } \
             await null; return log.join();",
        );
        assert_eq!(out, "true,true,image/png,true,true,true,true");
        let sent = sent();
        assert_eq!(sent.len(), 1, "{sent:?}");
        let mut expected = serde_json::json!({
            "metadata": { "title": "Song", "artist": "Band", "album": "", "artworkUrl": "https://x.test/l.png" },
            "playbackState": "playing",
            "actions": ["seekto"],
            "position": { "duration": 100.0, "position": 5.0, "playbackRate": 1.0 },
        });
        if !crate::media_session::LOADS_ARTWORK_URLS {
            expected["metadata"].as_object_mut().unwrap().remove("artworkUrl");
        }
        assert_eq!(sent[0].0, expected);
        let out = sk_runtime::testing::on_js_thread(|| {
            sk_runtime::JsRuntime::with(|rt| unsafe {
                super::super::deliver(
                    rt.raw(),
                    0,
                    true,
                    r#"{"kind":"mediasession","action":"seekto","seekTime":42}"#,
                    None,
                );
                let v = rt.ctx().eval("String(globalThis.seek)").unwrap();
                rt.ctx().to_rust_string(v).unwrap()
            })
        });
        assert_eq!(out, "seekto:42");
    }
}
