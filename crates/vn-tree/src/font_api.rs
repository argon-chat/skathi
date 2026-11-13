//! Native CSS Font Loading objects.
use crate::fonts::{self, Descriptors, Snapshot, Status};
use std::{cell::RefCell, collections::HashMap, ffi::c_void};
use vn_runtime::redjs::{
    context, ffi, glue,
    webapi::{domexception, events, util as u},
};
use vn_runtime::{host_fn, JsRuntime};

#[derive(Clone, Copy)]
struct Api {
    face: usize,
    set: usize,
    event: usize,
    root: u64,
    face_proto: u64,
    set_proto: u64,
    event_proto: u64,
    iterator: usize,
    values: u64,
    entries: u64,
    freeze: u64,
    weak_ctor: u64,
    weak_deref: u64,
    noop: u64,
}
thread_local! { static APIS: RefCell<HashMap<context::ContextId, Api>> = RefCell::new(HashMap::new()); }
unsafe fn api(g: *mut c_void) -> Api {
    let id = context::id_of(g).expect("registered JSC context");
    APIS.with(|s| *s.borrow().get(&id).expect("FontFace API installed"))
}
/// A Rust local is not an ownership edge in the JS heap.
struct Root(*mut c_void, u64);
impl Root {
    unsafe fn new(g: *mut c_void, v: u64) -> Self {
        ffi::redjs_protect(g, v);
        Self(g, v)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        unsafe {
            ffi::redjs_unprotect(self.0, self.1);
        }
    }
}
unsafe fn string(g: *mut c_void, v: u64) -> Result<String, ()> {
    let v = u::to_js_string(g, v)?;
    Ok(u::read_string(g, v).unwrap_or_default())
}
unsafe fn put(g: *mut c_void, bag: u64, name: &str, v: u64) {
    let _value = Root::new(g, v);
    u::define(g, bag, name, v, 0);
}
unsafe fn text(g: *mut c_void, bag: u64, name: &str) -> String {
    u::read_string(g, u::get(g, bag, name)).unwrap_or_default()
}
unsafe fn slot(g: *mut c_void, obj: u64, atom: usize) -> Option<u64> {
    if !u::is_cell(obj) {
        return None;
    }
    let mut out = u::UNDEF;
    (glue::rjs_get_own_data(g, obj, atom as *mut c_void, &mut out) == 1 && u::is_cell(out))
        .then_some(out)
}
unsafe fn face_bag(g: *mut c_void, obj: u64) -> Option<u64> {
    slot(g, obj, api(g).face)
}
unsafe fn set_bag(g: *mut c_void, obj: u64) -> Option<u64> {
    slot(g, obj, api(g).set)
}
unsafe fn face_id(g: *mut c_void, face: u64) -> u32 {
    face_bag(g, face)
        .and_then(|b| u::dec_f64(u::get(g, b, "id")))
        .unwrap_or(0.0) as u32
}
unsafe fn array(g: *mut c_void, values: &[u64]) -> u64 {
    let a = u::new_array(g);
    let _r = Root::new(g, a);
    for (i, v) in values.iter().enumerate() {
        u::set_i(g, a, i as u32, *v);
    }
    a
}
unsafe fn items(g: *mut c_void, a: u64) -> Vec<u64> {
    (0..u::array_len(g, a)).map(|i| u::get_i(g, a, i)).collect()
}
unsafe fn append(g: *mut c_void, a: u64, v: u64) {
    u::set_i(g, a, u::array_len(g, a), v);
}
unsafe fn promise(g: *mut c_void, bag: u64, name: &str) -> u64 {
    let (mut resolve, mut reject, mut p) = (0, 0, 0);
    ffi::redjs_promise_create(g, &mut resolve, &mut reject, &mut p);
    let (_p, _r, _j) = (Root::new(g, p), Root::new(g, resolve), Root::new(g, reject));
    put(g, bag, name, p);
    put(g, bag, &format!("{name}Resolve"), resolve);
    put(g, bag, &format!("{name}Reject"), reject);
    p
}
host_fn!(fn noop(_g,_args) { u::UNDEF });
#[cfg(test)]
host_fn!(fn test_gc(g,_args) { glue::rjs_gc(g,0);u::UNDEF });
unsafe fn construct(g: *mut c_void, ctor: u64, args: &[u64]) -> Result<u64, ()> {
    let mut out = u::UNDEF;
    if glue::rjs_construct(g, ctor, args.as_ptr(), args.len() as u32, &mut out) == 0 {
        Ok(out)
    } else {
        glue::rjs_throw(g, out);
        Err(())
    }
}
unsafe fn checked_create(g: *mut c_void, target: u64, default: u64) -> Result<u64, ()> {
    let proto = if target == u::UNDEF {
        default
    } else {
        u::try_get(g, target, "prototype")?
    };
    Ok(u::new_object(
        g,
        if glue::rjs_kind(g, proto) >= 8 {
            proto
        } else {
            default
        },
    ))
}
unsafe fn close_iterator(g: *mut c_void, it: u64) {
    let atom = u::atom(g, "return");
    let mut method = u::UNDEF;
    let rc = glue::rjs_get(g, it, atom, &mut method);
    glue::rjs_atom_free(g, atom);
    if rc == 0 && method != u::UNDEF && method != u::NULL {
        let _ = u::call(g, method, it, &[]);
    }
}
unsafe fn iterable(g: *mut c_void, value: u64) -> Result<(Vec<u64>, Vec<Root>), ()> {
    let mut method = u::UNDEF;
    if glue::rjs_get(g, value, api(g).iterator as *mut c_void, &mut method) != 0 {
        glue::rjs_throw(g, method);
        return Err(());
    }
    let it = match u::call(g, method, value, &[]) {
        Ok(v) => v,
        Err(e) => {
            glue::rjs_throw(g, e);
            return Err(());
        }
    };
    let _root = Root::new(g, it);
    let next = u::try_get(g, it, "next")?;
    let _next = Root::new(g, next);
    let mut out = Vec::new();
    let mut roots = Vec::new();
    loop {
        let step = match u::call(g, next, it, &[]) {
            Ok(v) => v,
            Err(e) => {
                glue::rjs_throw(g, e);
                return Err(());
            }
        };
        let _step = Root::new(g, step);
        if glue::rjs_kind(g, step) < 8 {
            u::throw_type_error(g, "Iterator result must be an object");
            return Err(());
        }
        if glue::rjs_to_boolean(g, u::try_get(g, step, "done")?) != 0 {
            return Ok((out, roots));
        }
        let value = u::try_get(g, step, "value")?;
        roots.push(Root::new(g, value));
        if face_bag(g, value).is_none() {
            close_iterator(g, it);
            u::throw_type_error(g, "FontFace sequence members must be FontFace objects");
            return Err(());
        }
        out.push(value);
        if out.len() > 1_000_000 {
            u::throw_type_error(g, "FontFace sequence is too large");
            return Err(());
        }
    }
}
const DESCRIPTORS: &[(&str, &str)] = &[
    ("family", ""),
    ("style", "normal"),
    ("weight", "normal"),
    ("stretch", "normal"),
    ("unicodeRange", "U+0-10FFFF"),
    ("display", "auto"),
    ("variant", "normal"),
    ("featureSettings", "normal"),
    ("variationSettings", "normal"),
    ("ascentOverride", "normal"),
    ("descentOverride", "normal"),
    ("lineGapOverride", "normal"),
];
unsafe fn descriptor_values(g: *mut c_void, family: String, dict: u64) -> Result<Vec<String>, ()> {
    let mut out = vec![family];
    for (name, default) in &DESCRIPTORS[1..] {
        let v = if dict == u::UNDEF || dict == u::NULL {
            u::UNDEF
        } else {
            u::try_get(g, dict, name)?
        };
        out.push(if v == u::UNDEF {
            (*default).into()
        } else {
            string(g, v)?
        });
    }
    Ok(out)
}
fn parsed(values: &[String]) -> Result<Descriptors, String> {
    let packed = values[1..=5]
        .iter()
        .map(|v| v.replace('|', ""))
        .collect::<Vec<_>>()
        .join("|");
    fonts::descriptors(&values[0], &packed)
}
fn snapshot_values(s: &Snapshot) -> Vec<String> {
    let mut v = DESCRIPTORS
        .iter()
        .map(|(_, v)| (*v).to_owned())
        .collect::<Vec<_>>();
    v[0] = s.desc.family.clone();
    v[1] = s.desc.style.clone();
    v[2] = s.desc.weight.clone();
    v[3] = s.desc.stretch.clone();
    v[4] = s.desc.range_text();
    v[5] = s.desc.display.clone();
    v
}
unsafe fn new_face(g: *mut c_void, new_target: u64, values: &[String], id: u32, css: bool) -> u64 {
    let a = api(g);
    let obj = match checked_create(g, new_target, a.face_proto) {
        Ok(v) => v,
        Err(()) => return u::UNDEF,
    };
    let _obj = Root::new(g, obj);
    let b = u::new_object(g, u::NULL);
    glue::rjs_define(g, obj, a.face as *mut c_void, b, 1 | 2 | 4);
    put(g, b, "id", u::enc_f64(id as f64));
    put(g, b, "css", u::enc_bool(css));
    put(g, b, "status", u::js_string(g, "unloaded"));
    for ((name, _), v) in DESCRIPTORS.iter().zip(values) {
        put(g, b, name, u::js_string(g, v));
    }
    let p = promise(g, b, "loaded");
    // Mark that original promise handled without changing its identity/result.
    let catch = u::get(g, p, "catch");
    let _ = u::call(g, catch, p, &[a.noop]);
    if id != 0 {
        u::set_i(g, u::get(g, a.root, "faces"), id, obj);
    }
    obj
}
unsafe fn canonical(g: *mut c_void, s: &Snapshot) -> u64 {
    let faces = u::get(g, api(g).root, "faces");
    let old = u::get_i(g, faces, s.id);
    if face_bag(g, old).is_some() {
        return old;
    }
    new_face(g, u::UNDEF, &snapshot_values(s), s.id, s.css)
}
// No untraced Rust vector is the only owner of a live set while a listener can collect.
unsafe fn for_sets(g: *mut c_void, mut visit: impl FnMut(u64)) {
    let a = api(g);
    let refs = u::get(g, a.root, "sets");
    let _refs = Root::new(g, refs);
    for weak in items(g, refs) {
        if let Ok(set) = u::call(g, a.weak_deref, weak, &[]) {
            if set_bag(g, set).is_some() {
                let _set = Root::new(g, set);
                visit(set);
            }
        }
    }
}
unsafe fn members(g: *mut c_void, set: u64) -> Vec<u64> {
    let Some(b) = set_bag(g, set) else {
        return Vec::new();
    };
    let mut out = items(g, u::get(g, b, "css"));
    for face in items(g, u::get(g, b, "added")) {
        if !out.contains(&face) {
            out.push(face);
        }
    }
    out
}
unsafe fn settle(g: *mut c_void, face: u64, status: &str, message: &str, syntax: bool) {
    let Some(b) = face_bag(g, face) else {
        return;
    };
    let old = text(g, b, "status");
    if old == "loaded" || old == "error" || status == "unloaded" || old == status {
        return;
    }
    put(g, b, "status", u::js_string(g, status));
    if status == "loading" {
        return;
    }
    let error = if status == "error" {
        domexception::create(
            g,
            if syntax {
                "SyntaxError"
            } else {
                "NetworkError"
            },
            message,
        )
    } else {
        u::UNDEF
    };
    put(g, b, "error", error);
    for_sets(g, |set| {
        if old == "loading" && members(g, set).contains(&face) {
            let sb = set_bag(g, set).unwrap();
            let key = if status == "loaded" { "done" } else { "failed" };
            let list = u::get(g, sb, key);
            if !items(g, list).contains(&face) {
                append(g, list, face);
            }
        }
    });
    let f = u::get(
        g,
        b,
        if status == "loaded" {
            "loadedResolve"
        } else {
            "loadedReject"
        },
    );
    let _f = Root::new(g, f);
    put(g, b, "loadedResolve", u::UNDEF);
    put(g, b, "loadedReject", u::UNDEF);
    let _ = u::call(
        g,
        f,
        u::UNDEF,
        &[if status == "loaded" { face } else { error }],
    );
}
unsafe fn load_face(g: *mut c_void, face: u64) -> u64 {
    let b = face_bag(g, face).unwrap();
    if text(g, b, "status") == "unloaded" {
        settle(g, face, "loading", "", false);
        fonts::load(face_id(g, face));
    }
    for_sets(g, |set| {
        if members(g, set).contains(&face) {
            state(g, set);
        }
    });
    u::get(g, b, "loaded")
}
unsafe fn event(g: *mut c_void, ty: &str, faces: &[u64]) -> u64 {
    let e = u::new_object(g, api(g).event_proto);
    let _e = Root::new(g, e);
    let ty = u::js_string(g, ty);
    let _ty = Root::new(g, ty);
    events::init_event(g, e, ty, u::UNDEF, false);
    let list = array(g, faces);
    let _list = Root::new(g, list);
    let _ = u::call(g, api(g).freeze, u::UNDEF, &[list]);
    glue::rjs_define(g, e, api(g).event as *mut c_void, list, 1 | 2 | 4);
    e
}
unsafe fn fire(g: *mut c_void, set: u64, ty: &str, faces: &[u64]) {
    let e = event(g, ty, faces);
    let _event = Root::new(g, e);
    events::dispatch(g, set, e);
}
unsafe fn loading(g: *mut c_void, set: u64) -> Vec<u64> {
    members(g, set)
        .into_iter()
        .filter(|f| face_bag(g, *f).is_some_and(|b| text(g, b, "status") == "loading"))
        .collect()
}
/// A retained pending promise must keep its owner alive even when script drops the FontFaceSet
/// itself.
unsafe fn refresh_active(g: *mut c_void, set: u64) {
    let Some(b) = set_bag(g, set) else {
        return;
    };
    let pending = u::get(g, b, "pending");
    let needed = !loading(g, set).is_empty()
        || u::get(g, b, "readyResolve") != u::UNDEF
        || (pending != u::UNDEF && u::array_len(g, pending) != 0);
    let root = api(g).root;
    let active = u::get(g, root, "active");
    let mut values = items(g, active);
    if needed {
        if !values.contains(&set) {
            append(g, active, set);
        }
    } else if values.contains(&set) {
        values.retain(|v| *v != set);
        put(g, root, "active", array(g, &values));
    }
}
unsafe fn state(g: *mut c_void, set: u64) {
    let Some(b) = set_bag(g, set) else {
        return;
    };
    refresh_active(g, set);
    let active = loading(g, set);
    let old = text(g, b, "status");
    if !active.is_empty() && old != "loading" {
        put(g, b, "status", u::js_string(g, "loading"));
        if u::get(g, b, "readyResolve") == u::UNDEF {
            promise(g, b, "ready");
        }
        fire(g, set, "loading", &active);
    } else if active.is_empty() && old == "loading" {
        put(g, b, "status", u::js_string(g, "loaded"));
        let done_array = u::get(g, b, "done");
        let failed_array = u::get(g, b, "failed");
        let (_done, _failed) = (Root::new(g, done_array), Root::new(g, failed_array));
        let done = items(g, done_array);
        let failed = items(g, failed_array);
        put(g, b, "done", u::new_array(g));
        put(g, b, "failed", u::new_array(g));
        fire(g, set, "loadingdone", &done);
        if !failed.is_empty() {
            fire(g, set, "loadingerror", &failed);
        }
    }
    let quiet = u::get(g, b, "document") != u::TRUE || fonts::quiet();
    if !quiet && u::get(g, b, "wasQuiet") == u::TRUE && u::get(g, b, "readyResolve") == u::UNDEF {
        promise(g, b, "ready");
    }
    put(g, b, "wasQuiet", u::enc_bool(quiet));
    if quiet && text(g, b, "status") == "loaded" && loading(g, set).is_empty() {
        let resolve = u::get(g, b, "readyResolve");
        if resolve != u::UNDEF {
            let _resolve = Root::new(g, resolve);
            put(g, b, "readyResolve", u::UNDEF);
            put(g, b, "readyReject", u::UNDEF);
            let _ = u::call(g, resolve, u::UNDEF, &[set]);
        }
    }
    refresh_active(g, set);
}
unsafe fn sync_css(g: *mut c_void, set: u64, snapshots: &[Snapshot]) {
    let Some(b) = set_bag(g, set) else {
        return;
    };
    if u::get(g, b, "document") != u::TRUE {
        return;
    }
    let css = snapshots
        .iter()
        .filter(|s| s.css)
        .map(|s| canonical(g, s))
        .collect::<Vec<_>>();
    for old in items(g, u::get(g, b, "css")) {
        if !css.contains(&old) {
            u::set_i(
                g,
                u::get(g, api(g).root, "faces"),
                face_id(g, old),
                u::UNDEF,
            );
        }
    }
    put(g, b, "css", array(g, &css));
}
unsafe fn fresh(g: *mut c_void, set: u64) {
    if set_bag(g, set).is_none() {
        return;
    }
    let snapshots = fonts::snapshots();
    sync_css(g, set, &snapshots);
    for s in &snapshots {
        let face = u::get_i(g, u::get(g, api(g).root, "faces"), s.id);
        if face_bag(g, face).is_some() {
            let message = match &s.status {
                Status::Error(e) => e.as_str(),
                _ => "",
            };
            settle(g, face, s.status.name(), message, false);
        }
    }
    state(g, set);
}
unsafe fn new_set(g: *mut c_void, new_target: u64, document: bool) -> u64 {
    let a = api(g);
    let obj = match checked_create(g, new_target, a.set_proto) {
        Ok(v) => v,
        Err(()) => return u::UNDEF,
    };
    let _obj = Root::new(g, obj);
    let b = u::new_object(g, u::NULL);
    glue::rjs_define(g, obj, a.set as *mut c_void, b, 1 | 2 | 4);
    glue::rjs_define(g, obj, u::state(g).s_listeners, u::new_array(g), 0);
    for key in ["css", "added", "done", "failed"] {
        put(g, b, key, u::new_array(g));
    }
    put(g, b, "wasQuiet", u::FALSE);
    put(g, b, "document", u::enc_bool(document));
    put(g, b, "status", u::js_string(g, "loaded"));
    promise(g, b, "ready");
    for key in ["onloading", "onloadingdone", "onloadingerror"] {
        u::define(g, obj, key, u::NULL, 0);
    }
    if let Ok(weak) = construct(g, a.weak_ctor, &[obj]) {
        append(g, u::get(g, a.root, "sets"), weak);
    }
    obj
}

host_fn!(fn face_construct(g,args) {
    if args.argc()<2{return u::throw_type_error(g,"FontFace requires a family and source");}
    let family=match string(g,args.arg(0).0){Ok(v)=>v,Err(())=>return u::UNDEF};
    let values=match descriptor_values(g,family,args.arg(2).0){Ok(v)=>v,Err(())=>return u::UNDEF};
    let bytes=u::bufferish(g,args.arg(1).0).map(|b|b.to_vec());
    let binary=bytes.is_some();
    let source=if binary {String::new()} else {match string(g,args.arg(1).0){Ok(v)=>v,Err(())=>return u::UNDEF}};
    let result=parsed(&values).and_then(|desc|if let Some(bytes)=bytes{Ok(fonts::create_bytes(desc,bytes))}else{fonts::create(desc,&source,false)});
    let id=result.as_ref().copied().unwrap_or(0);
    let face=new_face(g,args.this().0,&values,id,false);if face==u::UNDEF{return u::UNDEF;}
    match result{Ok(_)=>if binary{settle(g,face,"loading","",false)},Err(e)=>settle(g,face,"error",&e,true)}
    face
});
host_fn!(fn face_load(g,args){
    if face_bag(g,args.this().0).is_none(){return u::throw_type_error(g,"FontFace.load called on an incompatible receiver");}
    load_face(g,args.this().0)
});
macro_rules! face_getter {($fun:ident,$key:literal)=>{host_fn!(fn $fun(g,args){
    match face_bag(g,args.this().0){Some(b)=>u::get(g,b,$key),None=>u::throw_type_error(g,"Incompatible FontFace receiver")}
});};}
face_getter!(face_status, "status");
face_getter!(face_loaded, "loaded");
unsafe fn set_descriptor(g: *mut c_void, face: u64, key: &str, value: u64) -> u64 {
    if face_bag(g, face).is_none() {
        return u::throw_type_error(g, "Incompatible FontFace receiver");
    }
    let value = match string(g, value) {
        Ok(v) => v,
        Err(()) => return u::UNDEF,
    };
    let b = face_bag(g, face).unwrap();
    let mut values = DESCRIPTORS
        .iter()
        .map(|(n, _)| text(g, b, n))
        .collect::<Vec<_>>();
    let index = DESCRIPTORS.iter().position(|(n, _)| *n == key).unwrap();
    values[index] = value;
    let desc = if index <= 5 {
        match parsed(&values) {
            Ok(v) => Some(v),
            Err(e) => return domexception::throw(g, "SyntaxError", &e),
        }
    } else {
        None
    };
    put(g, b, key, u::js_string(g, &values[index]));
    if let Some(desc) = desc {
        fonts::set_descriptors(face_id(g, face), desc);
    }
    u::UNDEF
}
macro_rules! descriptor {($get:ident,$set:ident,$key:literal)=>{face_getter!($get,$key);host_fn!(fn $set(g,args){set_descriptor(g,args.this().0,$key,args.arg(0).0)});};}
descriptor!(get_family, set_family, "family");
descriptor!(get_style, set_style, "style");
descriptor!(get_weight, set_weight, "weight");
descriptor!(get_stretch, set_stretch, "stretch");
descriptor!(get_range, set_range, "unicodeRange");
descriptor!(get_display, set_display, "display");
descriptor!(get_variant, set_variant, "variant");
descriptor!(get_features, set_features, "featureSettings");
descriptor!(get_variations, set_variations, "variationSettings");
descriptor!(get_ascent, set_ascent, "ascentOverride");
descriptor!(get_descent, set_descent, "descentOverride");
descriptor!(get_gap, set_gap, "lineGapOverride");

host_fn!(fn set_construct(g,args){
    if args.argc()<1{return u::throw_type_error(g,"FontFaceSet requires an iterable");}
    let (values,_roots)=match iterable(g,args.arg(0).0){Ok(v)=>v,Err(())=>return u::UNDEF};
    if values.iter().any(|v|face_bag(g,*v).is_none()){return u::throw_type_error(g,"FontFaceSet members must be FontFace objects");}
    let set=new_set(g,args.this().0,false);if set==u::UNDEF{return u::UNDEF;}let _set=Root::new(g,set);
    for face in values{add(g,set,face);}
    state(g,set);set
});
unsafe fn add(g: *mut c_void, set: u64, face: u64) -> u64 {
    let Some(b) = set_bag(g, set) else {
        return u::throw_type_error(g, "Incompatible FontFaceSet receiver");
    };
    let Some(f) = face_bag(g, face) else {
        return u::throw_type_error(g, "FontFaceSet.add requires a FontFace");
    };
    if u::get(g, f, "css") == u::TRUE {
        return domexception::throw(
            g,
            "InvalidModificationError",
            "A CSS-connected FontFace cannot be added",
        );
    }
    if !members(g, set).contains(&face) {
        append(g, u::get(g, b, "added"), face);
        if u::get(g, b, "document") == u::TRUE {
            fonts::set_in_set(face_id(g, face), true);
        }
    }
    state(g, set);
    set
}
host_fn!(fn set_add(g,args){let set=args.this().0;fresh(g,set);add(g,set,args.arg(0).0)});
host_fn!(fn set_delete(g,args){
    let set=args.this().0;let Some(b)=set_bag(g,set) else{return u::throw_type_error(g,"Incompatible FontFaceSet receiver");};fresh(g,set);
    let face=args.arg(0).0;let mut added=items(g,u::get(g,b,"added"));let n=added.len();added.retain(|v|*v!=face);
    if added.len()==n{return u::FALSE;}
    put(g,b,"added",array(g,&added));
    if u::get(g,b,"document")==u::TRUE{fonts::set_in_set(face_id(g,face),false);}
    state(g,set);u::TRUE
});
host_fn!(fn set_clear(g,args){
    let set=args.this().0;let Some(b)=set_bag(g,set) else{return u::throw_type_error(g,"Incompatible FontFaceSet receiver");};fresh(g,set);
    let added=items(g,u::get(g,b,"added"));put(g,b,"added",u::new_array(g));
    if u::get(g,b,"document")==u::TRUE{for face in added{fonts::set_in_set(face_id(g,face),false);}}
    state(g,set);u::UNDEF
});
host_fn!(fn set_has(g,args){
    let set=args.this().0;if set_bag(g,set).is_none(){return u::throw_type_error(g,"Incompatible FontFaceSet receiver");}fresh(g,set);
    u::enc_bool(members(g,set).contains(&args.arg(0).0))
});
macro_rules! set_getter {($fun:ident,$key:literal)=>{host_fn!(fn $fun(g,args){
    let set=args.this().0;let Some(b)=set_bag(g,set) else{return u::throw_type_error(g,"Incompatible FontFaceSet receiver");};fresh(g,set);u::get(g,b,$key)
});};}
set_getter!(set_status, "status");
set_getter!(set_ready, "ready");
host_fn!(fn set_size(g,args){
    let set=args.this().0;if set_bag(g,set).is_none(){return u::throw_type_error(g,"Incompatible FontFaceSet receiver");}fresh(g,set);u::enc_f64(members(g,set).len() as f64)
});
unsafe fn iterator(g: *mut c_void, set: u64, entries: bool) -> u64 {
    if set_bag(g, set).is_none() {
        return u::throw_type_error(g, "Incompatible FontFaceSet receiver");
    }
    fresh(g, set);
    let values = members(g, set);
    let list = u::new_array(g);
    let _list = Root::new(g, list);
    for value in values {
        if entries {
            let pair = array(g, &[value, value]);
            let _pair = Root::new(g, pair);
            #[cfg(test)]
            glue::rjs_gc(g, 0);
            append(g, list, pair);
        } else {
            append(g, list, value);
        }
    }
    match u::call(g, api(g).values, list, &[]) {
        Ok(v) => v,
        Err(e) => {
            glue::rjs_throw(g, e);
            u::UNDEF
        }
    }
}
host_fn!(fn set_values(g,args){iterator(g,args.this().0,false)});
host_fn!(fn set_entries(g,args){iterator(g,args.this().0,true)});
host_fn!(fn set_foreach(g,args){
    let set=args.this().0;if set_bag(g,set).is_none(){return u::throw_type_error(g,"Incompatible FontFaceSet receiver");}fresh(g,set);
    let callback=args.arg(0).0;if glue::rjs_kind(g,callback)!=9{return u::throw_type_error(g,"FontFaceSet.forEach requires a callback");}
    let mut seen=Vec::new();
    loop{
        let next=members(g,set).into_iter().find(|v|!seen.contains(v));let Some(face)=next else{break;};seen.push(face);
        if let Err(e)=u::call(g,callback,args.arg(1).0,&[face,face,set]){glue::rjs_throw(g,e);return u::UNDEF;}
    }
    u::UNDEF
});
unsafe fn selected(
    g: *mut c_void,
    set: u64,
    font: u64,
    text_value: u64,
) -> Result<Option<Vec<u64>>, ()> {
    let font = string(g, font)?;
    let text = if text_value == u::UNDEF {
        " ".into()
    } else {
        string(g, text_value)?
    };
    fresh(g, set);
    let members = members(g, set);
    let ids = members.iter().map(|v| face_id(g, *v)).collect::<Vec<_>>();
    Ok(fonts::matching_for(&font, &text, Some(&ids)).map(|ids| {
        ids.into_iter()
            .filter_map(|id| members.iter().copied().find(|v| face_id(g, *v) == id))
            .collect()
    }))
}
host_fn!(fn set_check(g,args){
    if set_bag(g,args.this().0).is_none(){return u::throw_type_error(g,"Incompatible FontFaceSet receiver");}
    if args.argc()<1{return u::throw_type_error(g,"FontFaceSet.check requires a font");}
    let values=match selected(g,args.this().0,args.arg(0).0,args.arg(1).0){Ok(Some(v))=>v,Ok(None)=>return domexception::throw(g,"SyntaxError","Invalid font shorthand"),Err(())=>return u::UNDEF};
    u::enc_bool(values.iter().all(|v|text(g,face_bag(g,*v).unwrap(),"status")=="loaded"))
});
// It does not need JS closures, source evaluation or untraced Rust-held JS references.
unsafe fn finish_loads(g: *mut c_void, set: u64) {
    let b = set_bag(g, set).unwrap();
    let pending = u::get(g, b, "pending");
    let _pending = Root::new(g, pending);
    put(g, b, "pending", u::new_array(g));
    for job in items(g, pending) {
        let faces = items(g, u::get(g, job, "faces"));
        let failed = faces
            .iter()
            .find(|v| text(g, face_bag(g, **v).unwrap(), "status") == "error")
            .copied();
        if let Some(face) = failed {
            let reject = u::get(g, job, "resultReject");
            let error = u::get(g, face_bag(g, face).unwrap(), "error");
            let _ = u::call(g, reject, u::UNDEF, &[error]);
        } else if faces
            .iter()
            .all(|v| text(g, face_bag(g, *v).unwrap(), "status") == "loaded")
        {
            let _ = u::call(
                g,
                u::get(g, job, "resultResolve"),
                u::UNDEF,
                &[u::get(g, job, "faces")],
            );
        } else {
            append(g, u::get(g, b, "pending"), job);
        }
    }
    refresh_active(g, set);
}
host_fn!(fn set_load(g,args){
    let set=args.this().0;let Some(b)=set_bag(g,set) else{return u::throw_type_error(g,"Incompatible FontFaceSet receiver");};
    if args.argc()<1{return u::rejected_promise(g,1,"FontFaceSet.load requires a font");}
    let values=match selected(g,set,args.arg(0).0,args.arg(1).0){Ok(Some(v))=>v,Ok(None)=>return domexception::rejected(g,"SyntaxError","Invalid font shorthand"),Err(())=>return u::UNDEF};
    let job=u::new_object(g,u::NULL);let _job=Root::new(g,job);put(g,job,"faces",array(g,&values));let p=promise(g,job,"result");
    let mut pending=u::get(g,b,"pending");if pending==u::UNDEF{pending=u::new_array(g);put(g,b,"pending",pending);}append(g,pending,job);
    for face in values{load_face(g,face);}
    finish_loads(g,set);p
});
host_fn!(fn load_event_construct(g,args){
    if args.argc()<1{return u::throw_type_error(g,"FontFaceSetLoadEvent requires a type");}
    let ty=match string(g,args.arg(0).0){Ok(v)=>v,Err(())=>return u::UNDEF};
    let init=args.arg(1).0;
    let (faces,_roots)=if init==u::UNDEF || init==u::NULL{(Vec::new(),Vec::new())}else{
        let v=match u::try_get(g,init,"fontfaces"){Ok(v)=>v,Err(())=>return u::UNDEF};
        if v==u::UNDEF{(Vec::new(),Vec::new())}else{match iterable(g,v){Ok(v)=>v,Err(())=>return u::UNDEF}}
    };
    if faces.iter().any(|v|face_bag(g,*v).is_none()){return u::throw_type_error(g,"fontfaces must contain FontFace objects");}
    let obj=match checked_create(g,args.this().0,api(g).event_proto){Ok(v)=>v,Err(())=>return u::UNDEF};let _obj=Root::new(g,obj);
    let ty=u::js_string(g,&ty);let _ty=Root::new(g,ty);events::init_event(g,obj,ty,init,false);
    let list=array(g,&faces);let _list=Root::new(g,list);let _=u::call(g,api(g).freeze,u::UNDEF,&[list]);
    glue::rjs_define(g,obj,api(g).event as *mut c_void,list,1|2|4);obj
});
host_fn!(fn event_faces(g,args){match slot(g,args.this().0,api(g).event){Some(v)=>v,None=>u::throw_type_error(g,"Incompatible FontFaceSetLoadEvent receiver")}});
host_fn!(fn document_fonts(g,_args){u::get(g,api(g).root,"document")});
host_fn!(fn attach(g,args){
    let doc=args.arg(0).0;if !u::is_cell(doc){return u::throw_type_error(g,"Font API attachment requires a document");}
    let a=api(g);let mut set=u::get(g,a.root,"document");
    if set==u::UNDEF{set=new_set(g,u::UNDEF,true);put(g,a.root,"document",set);}
    let name=u::atom(g,"fonts");glue::rjs_define_accessor(g,doc,name,u::make_fn(g,"get fonts",0,document_fonts),u::UNDEF,0);glue::rjs_atom_free(g,name);
    fresh(g,set);set
});

pub(crate) fn install(rt: &JsRuntime) {
    unsafe {
        let g = rt.raw();
        let id = context::ensure(g);
        if APIS.with(|s| s.borrow().contains_key(&id)) {
            return;
        }
        let face = u::define_class(
            g,
            &u::ClassSpec {
                name: "FontFace",
                arity: 2,
                construct: Some(face_construct),
                parent_proto: 0,
                methods: &[("load", 0, face_load)],
                getters: &[("status", face_status), ("loaded", face_loaded)],
                accessors: &[
                    ("family", get_family, set_family),
                    ("style", get_style, set_style),
                    ("weight", get_weight, set_weight),
                    ("stretch", get_stretch, set_stretch),
                    ("unicodeRange", get_range, set_range),
                    ("display", get_display, set_display),
                    ("variant", get_variant, set_variant),
                    ("featureSettings", get_features, set_features),
                    ("variationSettings", get_variations, set_variations),
                    ("ascentOverride", get_ascent, set_ascent),
                    ("descentOverride", get_descent, set_descent),
                    ("lineGapOverride", get_gap, set_gap),
                ],
            },
        );
        let set = u::define_class(
            g,
            &u::ClassSpec {
                name: "FontFaceSet",
                arity: 1,
                construct: Some(set_construct),
                parent_proto: u::state(g).event_target.proto,
                methods: &[
                    ("add", 1, set_add),
                    ("delete", 1, set_delete),
                    ("clear", 0, set_clear),
                    ("has", 1, set_has),
                    ("forEach", 1, set_foreach),
                    ("values", 0, set_values),
                    ("keys", 0, set_values),
                    ("entries", 0, set_entries),
                    ("check", 1, set_check),
                    ("load", 1, set_load),
                ],
                getters: &[
                    ("size", set_size),
                    ("status", set_status),
                    ("ready", set_ready),
                ],
                accessors: &[],
            },
        );
        let event = u::define_class(
            g,
            &u::ClassSpec {
                name: "FontFaceSetLoadEvent",
                arity: 1,
                construct: Some(load_event_construct),
                parent_proto: u::state(g).event.proto,
                methods: &[],
                getters: &[("fontfaces", event_faces)],
                accessors: &[],
            },
        );
        let root = u::new_object(g, u::NULL);
        let _root = Root::new(g, root);
        put(g, root, "faces", u::new_array(g));
        put(g, root, "sets", u::new_array(g));
        put(g, root, "active", u::new_array(g));
        let global = glue::rjs_global(g);
        let root_atom = u::atom_priv(g, "vn#fontsRoot");
        glue::rjs_define(g, global, root_atom, root, 1 | 2 | 4);
        let iterator = glue::rjs_atom_wellknown(g, 0);
        glue::rjs_define(g, set.proto, iterator, u::get(g, set.proto, "values"), 2);
        u::define(g, set.proto, "keys", u::get(g, set.proto, "values"), 2);
        let array_proto = u::get(g, u::get(g, global, "Array"), "prototype");
        let weak_ctor = u::get(g, global, "WeakRef");
        let weak_deref = u::get(g, u::get(g, weak_ctor, "prototype"), "deref");
        let a = Api {
            face: u::atom_priv(g, "vn#fontFace") as usize,
            set: u::atom_priv(g, "vn#fontSet") as usize,
            event: u::atom_priv(g, "vn#fontEvent") as usize,
            root,
            face_proto: face.proto,
            set_proto: set.proto,
            event_proto: event.proto,
            iterator: iterator as usize,
            values: u::get(g, array_proto, "values"),
            entries: u::get(g, array_proto, "entries"),
            freeze: u::get(g, u::get(g, global, "Object"), "freeze"),
            weak_ctor,
            weak_deref,
            noop: u::make_fn(g, "fontPromiseHandled", 0, noop),
        };
        for (n, v) in [
            ("values", a.values),
            ("entries", a.entries),
            ("freeze", a.freeze),
            ("weakCtor", a.weak_ctor),
            ("weakDeref", a.weak_deref),
            ("noop", a.noop),
        ] {
            put(g, root, n, v);
        }
        APIS.with(|s| s.borrow_mut().insert(id, a));
        rt.register("__vn_fonts_attach", 1, attach as *const c_void);
    }
}

/// Called only inside the frame's JS wave.
pub(crate) fn deliver(rt: &JsRuntime, changes: &[(u32, &'static str, String)]) {
    unsafe {
        let g = rt.raw();
        let Some(id) = context::id_of(g) else {
            return;
        };
        if !APIS.with(|s| s.borrow().contains_key(&id)) {
            return;
        }
        let snapshots = fonts::snapshots();
        for_sets(g, |set| sync_css(g, set, &snapshots));
        for (id, status, message) in changes {
            if *id != 0 {
                let face = u::get_i(g, u::get(g, api(g).root, "faces"), *id);
                settle(g, face, status, message, false);
            } else if *status == "busy" {
                for_sets(g, |set| {
                    let b = set_bag(g, set).unwrap();
                    if u::get(g, b, "document") == u::TRUE {
                        if u::get(g, b, "readyResolve") == u::UNDEF {
                            promise(g, b, "ready");
                        }
                        put(g, b, "wasQuiet", u::FALSE);
                    }
                });
            }
            for_sets(g, |set| {
                state(g, set);
                finish_loads(g, set);
            });
        }
        for_sets(g, |set| {
            fresh(g, set);
            finish_loads(g, set);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn js(rt: &JsRuntime, source: &str) {
        if let Err(e) = rt.eval(&format!("(()=>{{{source}\n}})()"), "font-native-test") {
            panic!("{}\n{}", e.header, e.stack);
        }
    }
    fn setup(rt: &JsRuntime) {
        install(rt);
        rt.register("__font_gc", 0, test_gc as *const c_void);
        fonts::note_sheet("");
        fonts::test_quiet(true);
        deliver(rt, &[]);
        js(
            rt,
            r#"globalThis.fontDoc={};globalThis.fontSet=__vn_fonts_attach(fontDoc);fontSet.clear();
            globalThis.fassert=(ok,message)=>{if(!ok)throw Error(message||'font assertion');};"#,
        );
        unsafe {
            let set = rt.ctx().global_get("fontSet").unwrap().0;
            glue::rjs_define(
                rt.raw(),
                set,
                u::state(rt.raw()).s_listeners,
                u::new_array(rt.raw()),
                0,
            );
            for key in ["onloading", "onloadingdone", "onloadingerror"] {
                put(rt.raw(), set, key, u::NULL);
            }
        }
        rt.ctx().drain_microtasks();
    }
    fn complete(rt: &JsRuntime, name: &str, status: Status) {
        unsafe {
            let face = rt.ctx().global_get(name).unwrap().0;
            let id = face_id(rt.raw(), face);
            let message = match &status {
                Status::Error(e) => e.clone(),
                _ => String::new(),
            };
            let state = status.name();
            fonts::test_status(id, status);
            deliver(rt, &[(id, state, message)]);
            rt.ctx().drain_microtasks();
        }
    }
    fn quiet(rt: &JsRuntime, on: bool) {
        fonts::test_quiet(on);
        deliver(rt, &[(0, if on { "quiet" } else { "busy" }, String::new())]);
        rt.ctx().drain_microtasks();
    }
    #[test]
    fn native_classes_attach_a_stable_document_set() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    r#"fassert(fontDoc.fonts===fontSet&&__vn_fonts_attach({})===fontSet);
            fassert(fontSet instanceof EventTarget);fassert(String(fontSet)==='[object FontFaceSet]');
            fassert(typeof FontFace==='function'&&typeof FontFaceSet==='function'&&typeof FontFaceSetLoadEvent==='function');
            fassert(FontFaceSet.prototype.keys===FontFaceSet.prototype.values);
            fassert(FontFaceSet.prototype[Symbol.iterator]===FontFaceSet.prototype.values);"#,
                );
            })
        });
    }
    #[test]
    fn css_faces_are_lazy_and_ready_waits_for_a_quiet_layout() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                quiet(rt, false);
                fonts::note_sheet(
                    r#"@font-face{font-family:NativeLato;src:url(lato.ttf);font-weight:700}
            @font-face{font-family:NativePUA;src:url(pua.ttf);unicode-range:U+E000-E0FF}"#,
                );
                js(
                    rt,
                    r#"globalThis.lato=[...fontSet][0];globalThis.pua=[...fontSet][1];globalThis.eventsSeen=[];globalThis.wasReady=false;
            fontSet.ready.then(v=>{fassert(v===fontSet);wasReady=true});
            for(const type of ['loading','loadingdone'])fontSet.addEventListener(type,e=>eventsSeen.push([type,e.fontfaces.map(f=>f.family)]));
            fassert(fontSet.size===2&&lato.weight==='700'&&lato.status==='unloaded'&&pua.unicodeRange==='U+E000-E0FF');
            fassert(lato instanceof FontFace&&fontSet.status==='loaded');
            fassert(lato.load()===lato.loaded&&lato.status==='loading'&&fontSet.status==='loading');"#,
                );
                complete(rt, "lato", Status::Loaded);
                js(
                    rt,
                    "fassert(lato.status==='loaded'&&!wasReady&&pua.status==='unloaded');",
                );
                quiet(rt, true);
                js(rt,"fassert(wasReady);fassert(JSON.stringify(eventsSeen)==='[[\"loading\",[\"NativeLato\"]],[\"loadingdone\",[\"NativeLato\"]]]');");
            })
        });
    }
    #[test]
    fn matching_and_load_use_native_family_and_unicode_ranges() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                fonts::note_sheet(
                    r#"@font-face{font-family:LoadLato;src:url(lato.ttf)}
            @font-face{font-family:LoadPUA;src:url(pua.ttf);unicode-range:U+E000-E0FF}"#,
                );
                js(
                    rt,
                    r#"globalThis.loadLato=[...fontSet][0];globalThis.loadPua=[...fontSet][1];globalThis.loadResult=null;globalThis.badName='';
            fassert(!fontSet.check('16px LoadLato')&&fontSet.check('16px MissingFont'));
            fassert(fontSet.check('16px LoadPUA','abc'));
            try{fontSet.check('LoadLato');throw Error('accepted bad font')}catch(e){fassert(e.name==='SyntaxError')}
            fontSet.load('LoadLato').catch(e=>badName=e.name);
            fontSet.load('16px LoadPUA','\ue010').then(v=>loadResult=v);"#,
                );
                complete(rt, "loadPua", Status::Loaded);
                js(rt,"fassert(badName==='SyntaxError'&&loadResult.length===1&&loadResult[0]===loadPua);fassert(fontSet.check('16px LoadPUA','\\ue010'));".replace("\\ue010","\u{e010}").as_str());
            })
        });
    }
    #[test]
    fn script_faces_join_after_loading_and_report_network_failure_once() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    r#"globalThis.brand=new FontFace('Brand','url(brand.woff2)',{weight:'300 700',style:'italic'});
            fassert(brand.status==='unloaded'&&!fontSet.has(brand));globalThis.brandLoaded=brand.load();
            fassert(brandLoaded===brand.loaded&&brand.load()===brandLoaded);brandLoaded.then(f=>fontSet.add(f));"#,
                );
                complete(rt, "brand", Status::Loaded);
                js(
                    rt,
                    r#"fassert(fontSet.has(brand)&&fontSet.delete(brand)&&!fontSet.has(brand));
            globalThis.gone=new FontFace('Gone','url(404.woff2)');fontSet.add(gone);globalThis.errorCount=0;globalThis.errorName='';
            fontSet.onloadingerror=e=>{errorCount++;fassert(e.fontfaces[0]===gone)};
            gone.load().catch(e=>errorName=e.name);fassert(fontSet.status==='loading');"#,
                );
                complete(rt, "gone", Status::Error("HTTP 404 Not Found".into()));
                js(
                    rt,
                    "fassert(gone.status==='error'&&errorName==='NetworkError'&&errorCount===1);",
                );
            })
        });
    }
    #[test]
    fn binary_sources_copy_only_the_view_without_blob_urls() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    r#"globalThis.bytesBacking=new Uint8Array([99,0,1,0,0,88]);
            globalThis.bytesFace=new FontFace('Bytes',new DataView(bytesBacking.buffer,1,4));
            fassert(bytesFace.status==='loading');bytesBacking.fill(7);"#,
                );
                let id = unsafe { face_id(rt.raw(), rt.ctx().global_get("bytesFace").unwrap().0) };
                assert_eq!(fonts::test_bytes(id), Some(vec![0, 1, 0, 0]));
                complete(rt, "bytesFace", Status::Loaded);
                js(rt, "fassert(bytesFace.status==='loaded');");
            })
        });
    }
    #[test]
    fn malformed_sources_and_descriptors_reject_the_stable_loaded_promise() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    r#"globalThis.invalidFace=new FontFace('X','nonsense(');globalThis.syntaxName='';
            fassert(invalidFace.status==='error'&&invalidFace.load()===invalidFace.loaded);
            invalidFace.loaded.catch(e=>syntaxName=e.name);
            try{new FontFace('one');throw Error('accepted missing source')}catch(e){fassert(e instanceof TypeError)}
            globalThis.invalidStyle=new FontFace('X','url(x.ttf)',{style:'bananas'});fassert(invalidStyle.status==='error');
            const sentinel={};try{new FontFace('X','url(x.ttf)',{get weight(){throw sentinel}});throw Error('getter ignored')}catch(e){fassert(e===sentinel)}"#,
                );
                rt.ctx().drain_microtasks();
                js(rt, "fassert(syntaxName==='SyntaxError');");
            })
        });
    }
    #[test]
    fn stylesheet_changes_are_synchronous_and_busy_renews_ready() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                quiet(rt, true);
                js(rt, "globalThis.oldReady=fontSet.ready;");
                quiet(rt, false);
                js(rt,"globalThis.newReady=fontSet.ready;fassert(newReady!==oldReady);globalThis.againReady=false;newReady.then(()=>againReady=true);");
                fonts::note_sheet("@font-face{font-family:Fresh;src:url(fresh.ttf)}");
                js(
                    rt,
                    "fassert(fontSet.size===1&&[...fontSet][0].family==='Fresh'&&!againReady);",
                );
                quiet(rt, true);
                js(rt, "fassert(againReady);");
            })
        });
    }
    #[test]
    fn css_membership_and_canonical_identity_survive_stylesheet_edits() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                fonts::note_sheet("@font-face{font-family:Keep;src:url(keep.ttf)}");
                js(
                    rt,
                    r#"globalThis.keepFace=[...fontSet][0];fassert(!fontSet.delete(keepFace));
            try{fontSet.add(keepFace);throw Error('added CSS face')}catch(e){fassert(e.name==='InvalidModificationError')}"#,
                );
                fonts::note_sheet("@font-face{font-family:Keep;src:url(keep.ttf)}@font-face{font-family:Next;src:url(next.ttf)}");
                js(rt,"fassert(fontSet.size===2&&[...fontSet][0]===keepFace&&[...fontSet][1].family==='Next');");
            })
        });
    }
    #[test]
    fn descriptors_independent_sets_gc_and_reentrant_listeners() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    r#"globalThis.gcFace=new FontFace('Before','url(gc.ttf)',{weight:400});gcFace.family='After';gcFace.weight='700';
            fassert(gcFace.family==='After'&&gcFace.weight==='700');
            try{gcFace.style='bad-style';throw Error('accepted style')}catch(e){fassert(e.name==='SyntaxError'&&gcFace.style==='normal')}
            globalThis.otherSet=new FontFaceSet([gcFace,gcFace]);fassert(otherSet.size===1&&!fontSet.has(gcFace));
            fassert(otherSet.check('16px Before')&&!otherSet.check('16px After'));
            globalThis.gcEvents=0;otherSet.addEventListener('loading',()=>{gcEvents++;__font_gc();fassert(otherSet.has(gcFace));otherSet.add(gcFace);});
            const ev=new FontFaceSetLoadEvent('custom',{fontfaces:[gcFace]});
            fassert(ev instanceof Event&&ev.fontfaces[0]===gcFace&&Object.isFrozen(ev.fontfaces));
            const pairs=[...otherSet.entries()];fassert(pairs[0][0]===gcFace&&pairs[0][1]===gcFace);
            otherSet.addEventListener('loading',e=>fassert(e.fontfaces[0]===gcFace));
            gcFace.load();"#,
                );
                rt.ctx().gc();
                complete(rt, "gcFace", Status::Loaded);
                rt.ctx().gc();
                js(rt,"fassert(gcFace.status==='loaded'&&otherSet.size===1&&gcEvents===1&&otherSet.check('16px After'));");
            })
        });
    }
    #[test]
    fn same_tick_css_failure_preserves_loading_done_error_order() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                fonts::note_sheet("@font-face{font-family:SameTick;src:url(missing.ttf)}");
                js(
                    rt,
                    r#"globalThis.sameTick=[...fontSet][0];globalThis.tickEvents=[];
            for(const type of ['loading','loadingdone','loadingerror'])fontSet.addEventListener(type,e=>tickEvents.push(type));"#,
                );
                let id = unsafe { face_id(rt.raw(), rt.ctx().global_get("sameTick").unwrap().0) };
                fonts::test_status(id, Status::Error("missing".into()));
                deliver(
                    rt,
                    &[
                        (id, "loading", String::new()),
                        (id, "error", "missing".into()),
                    ],
                );
                js(rt,"fassert(JSON.stringify(tickEvents)==='[\"loading\",\"loadingdone\",\"loadingerror\"]');");
            })
        });
    }

    #[test]
    fn iterable_conversion_closes_at_the_first_invalid_face_and_preserves_newtarget_errors() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    r#"globalThis.iteratorCalls=0;globalThis.iteratorClosed=false;
            const initial={ [Symbol.iterator](){return this},next(){iteratorCalls++;return {done:false,value:{}}},return(){iteratorClosed=true;return {done:true}}};
            try{new FontFaceSet(initial);throw Error('invalid face accepted')}catch(e){fassert(e instanceof TypeError)}
            fassert(iteratorCalls===1&&iteratorClosed);
            const sentinel={};const target=new Proxy(function(){},{get(object,key){if(key==='prototype')throw sentinel;return Reflect.get(object,key)}});
            for(const [ctor,args] of [[FontFace,['X','url(x.ttf)']],[FontFaceSet,[[]]],[FontFaceSetLoadEvent,['x']]]){
                try{Reflect.construct(ctor,args,target);throw Error('prototype throw swallowed')}catch(e){fassert(e===sentinel)}
            }
            const e=new FontFaceSetLoadEvent('x',{get bubbles(){__font_gc();return true},fontfaces:[]});fassert(e.bubbles);
        "#,
                );
            })
        });
    }
    #[test]
    fn retained_promises_keep_an_independent_loading_set_alive() {
        vn_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    r#"globalThis.pendingFace=new FontFace('PromiseOnly','url(pending.ttf)');
            globalThis.onlyLoadResult=null;globalThis.onlyReadyResult=null;
            (()=>{const set=new FontFaceSet([pendingFace]);globalThis.onlyLoad=set.load('16px PromiseOnly');onlyLoad.then(v=>onlyLoadResult=v)})();
            (()=>{const set=new FontFaceSet([pendingFace]);globalThis.onlyReady=set.ready;onlyReady.then(v=>onlyReadyResult=v)})();
        "#,
                );
                rt.ctx().gc();
                rt.ctx().drain_microtasks();
                rt.ctx().gc();
                complete(rt, "pendingFace", Status::Loaded);
                js(rt,"fassert(onlyLoadResult.length===1&&onlyLoadResult[0]===pendingFace);fassert(onlyReadyResult instanceof FontFaceSet&&onlyReadyResult.check('16px PromiseOnly'));");
            })
        });
    }
}
