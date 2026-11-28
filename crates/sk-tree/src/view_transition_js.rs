//! `document.startViewTransition`, `document.activeViewTransition` and `ViewTransition`; the
//! transition itself runs in `crate::view_transition`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;

use sk_runtime::host::{deferred, error, Settler};
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util as u;
use sk_runtime::{host_fn, JsRuntime};

use crate::view_transition::{self as vt, Note, Reason};

const KIND_OBJECT: u32 = 8;
const KIND_FUNCTION: u32 = 9;

struct Pending {
    obj: u64,
    update: u64,
    update_done: Option<Settler>,
    ready: Option<Settler>,
    finished: Option<Settler>,
    /// The update callback's outcome: `Err` holds its rejection reason.
    outcome: Option<Result<(), u64>>,
    ended: bool,
}

#[derive(Clone, Copy, Default)]
struct Api {
    proto: u64,
    bag: usize,
    promise: u64,
    promise_resolve: u64,
    set: u64,
    array_from: u64,
    fulfilled: u64,
    rejected: u64,
    noop: u64,
}

thread_local! {
    static API: Cell<Api> = Cell::new(Api::default());
    static PENDING: RefCell<HashMap<u32, Pending>> = RefCell::new(HashMap::new());
}

fn api() -> Api {
    API.with(Cell::get)
}

unsafe fn is_function(g: *mut c_void, v: u64) -> bool {
    unsafe { u::is_cell(v) && glue::rjs_kind(g, v) == KIND_FUNCTION }
}

unsafe fn is_object(g: *mut c_void, v: u64) -> bool {
    unsafe { u::is_cell(v) && glue::rjs_kind(g, v) >= KIND_OBJECT }
}

unsafe fn bag_of(g: *mut c_void, obj: u64) -> Option<u64> {
    unsafe {
        if !u::is_cell(obj) {
            return None;
        }
        let mut out = u::UNDEF;
        (glue::rjs_get_own_data(g, obj, api().bag as *mut c_void, &mut out) == 1 && u::is_cell(out))
            .then_some(out)
    }
}

unsafe fn seq_of(g: *mut c_void, obj: u64) -> Option<u32> {
    unsafe { bag_of(g, obj).and_then(|b| u::dec_f64(u::get(g, b, "seq")).map(|s| s as u32)) }
}

/// `types` as strings, or the exception reading them threw.
unsafe fn read_types(g: *mut c_void, list: u64) -> Result<Vec<String>, ()> {
    unsafe {
        if list == u::UNDEF || list == u::NULL {
            return Ok(Vec::new());
        }
        let a = api();
        let array = match u::call(g, a.array_from, u::UNDEF, &[list]) {
            Ok(v) => v,
            Err(e) => {
                glue::rjs_throw(g, e);
                return Err(());
            }
        };
        let mut out = Vec::new();
        for i in 0..u::array_len(g, array) {
            let s = u::to_js_string(g, u::get_i(g, array, i))?;
            out.push(u::read_string(g, s).unwrap_or_default());
        }
        Ok(out)
    }
}

unsafe fn construct(g: *mut c_void, ctor: u64, args: &[u64]) -> u64 {
    unsafe {
        let mut out = u::UNDEF;
        if glue::rjs_construct(g, ctor, args.as_ptr(), args.len() as u32, &mut out) == 0 {
            out
        } else {
            u::UNDEF
        }
    }
}

host_fn!(fn start(g, args) {
    unsafe {
        let arg = args.arg(0).0;
        let (update, types) = if is_function(g, arg) {
            (arg, u::UNDEF)
        } else if arg == u::UNDEF || arg == u::NULL {
            (u::UNDEF, u::UNDEF)
        } else if is_object(g, arg) {
            let update = match u::try_get(g, arg, "update") {
                Ok(v) => v,
                Err(()) => return u::UNDEF,
            };
            let types = match u::try_get(g, arg, "types") {
                Ok(v) => v,
                Err(()) => return u::UNDEF,
            };
            if update != u::UNDEF && !is_function(g, update) {
                return u::throw_type_error(g, "Failed to execute 'startViewTransition' on 'Document': The 'update' member is not a function.");
            }
            (update, types)
        } else {
            return u::throw_type_error(g, "Failed to execute 'startViewTransition' on 'Document': The provided value is neither a function nor an options object.");
        };
        let Ok(types) = read_types(g, types) else { return u::UNDEF };
        let this = args.this().0;
        let hidden = u::is_cell(this) && u::get(g, this, "hidden") == u::TRUE;
        let seq = vt::start(types.clone(), hidden);

        let a = api();
        let (update_done, update_done_settler) = deferred(g);
        let (ready, ready_settler) = deferred(g);
        let (finished, finished_settler) = deferred(g);
        let _ = u::call(g, u::get(g, ready, "then"), ready, &[u::UNDEF, a.noop]);
        let list = u::new_array(g);
        for (i, t) in types.iter().enumerate() {
            u::set_i(g, list, i as u32, u::js_string(g, t));
        }
        let set = construct(g, a.set, &[list]);
        let obj = u::new_object(g, a.proto);
        let bag = u::new_object(g, u::NULL);
        u::define(g, bag, "seq", u::enc_f64(seq as f64), 0);
        u::define(g, bag, "updateCallbackDone", update_done, 0);
        u::define(g, bag, "ready", ready, 0);
        u::define(g, bag, "finished", finished, 0);
        u::define(g, bag, "types", set, 0);
        glue::rjs_define(g, obj, a.bag as *mut c_void, bag, 1 | 2 | 4);
        redjs_protect(g, obj);
        if update != u::UNDEF {
            redjs_protect(g, update);
        }
        PENDING.with(|p| {
            p.borrow_mut().insert(seq, Pending {
                obj,
                update,
                update_done: Some(update_done_settler),
                ready: Some(ready_settler),
                finished: Some(finished_settler),
                outcome: None,
                ended: false,
            })
        });
        obj
    }
});

host_fn!(fn active(_g, _args) {
    vt::active_seq()
        .and_then(|seq| PENDING.with(|p| p.borrow().get(&seq).map(|r| r.obj)))
        .unwrap_or(u::NULL)
});

macro_rules! field {
    ($fun:ident, $key:literal) => {
        host_fn!(fn $fun(g, args) {
            unsafe {
                match bag_of(g, args.this().0) {
                    Some(b) => u::get(g, b, $key),
                    None => u::throw_type_error(g, "Illegal invocation"),
                }
            }
        });
    };
}
field!(update_callback_done, "updateCallbackDone");
field!(ready, "ready");
field!(finished, "finished");
field!(types, "types");

host_fn!(fn skip_transition(g, args) {
    unsafe {
        match seq_of(g, args.this().0) {
            Some(seq) => {
                vt::skip(seq, Reason::Abort);
                u::UNDEF
            }
            None => u::throw_type_error(g, "Illegal invocation"),
        }
    }
});

host_fn!(fn update_fulfilled(g, args) {
    unsafe {
        if let Some(seq) = u::dec_f64(args.arg(0).0) {
            on_update(g, seq as u32, Ok(()));
        }
    }
    u::UNDEF
});

host_fn!(fn update_rejected(g, args) {
    unsafe {
        if let Some(seq) = u::dec_f64(args.arg(0).0) {
            on_update(g, seq as u32, Err(args.arg(1).0));
        }
    }
    u::UNDEF
});

host_fn!(fn noop(_g, _args) { u::UNDEF });

host_fn!(fn attach(g, args) {
    unsafe {
        let doc = args.arg(0).0;
        if u::is_cell(doc) {
            sk_runtime::host::method(g, doc, "startViewTransition", 0, start);
            sk_runtime::host::getter(g, doc, "activeViewTransition", active);
        }
    }
    u::UNDEF
});

unsafe fn on_update(g: *mut c_void, seq: u32, outcome: Result<(), u64>) {
    unsafe {
        let settler = PENDING.with(|p| {
            let mut p = p.borrow_mut();
            let r = p.get_mut(&seq)?;
            if let Err(reason) = outcome {
                redjs_protect(g, reason);
            }
            r.outcome = Some(outcome);
            r.update_done.take()
        });
        if let Some(s) = settler {
            match outcome {
                Ok(()) => s.resolve(g, u::UNDEF),
                Err(reason) => s.reject(g, reason),
            }
        }
        vt::update_done(seq, outcome.is_ok());
        handle_all(g);
        try_finish(g, seq);
    }
}

unsafe fn try_finish(g: *mut c_void, seq: u32) {
    unsafe {
        let done = PENDING.with(|p| {
            let mut p = p.borrow_mut();
            let r = p.get(&seq)?;
            if !r.ended || r.outcome.is_none() {
                return None;
            }
            p.remove(&seq)
        });
        let Some(mut r) = done else { return };
        if let Some(s) = r.finished.take() {
            match r.outcome {
                Some(Err(reason)) => s.reject(g, reason),
                _ => s.resolve(g, u::UNDEF),
            }
        }
        for s in [r.update_done.take(), r.ready.take()].into_iter().flatten() {
            s.resolve(g, u::UNDEF);
        }
        if let Some(Err(reason)) = r.outcome {
            redjs_unprotect(g, reason);
        }
        if r.update != u::UNDEF {
            redjs_unprotect(g, r.update);
        }
        redjs_unprotect(g, r.obj);
    }
}

unsafe fn call_update(g: *mut c_void, seq: u32) {
    unsafe {
        let Some(update) = PENDING.with(|p| p.borrow().get(&seq).map(|r| r.update)) else {
            return;
        };
        if update == u::UNDEF {
            on_update(g, seq, Ok(()));
            return;
        }
        let a = api();
        let promise = u::call(g, update, u::UNDEF, &[])
            .and_then(|v| u::call(g, a.promise_resolve, a.promise, &[v]));
        match promise {
            Ok(p) => {
                let id = u::enc_f64(seq as f64);
                let bind = |f: u64| {
                    u::call(g, u::get(g, f, "bind"), f, &[u::UNDEF, id]).unwrap_or(u::UNDEF)
                };
                let (ok, err) = (bind(a.fulfilled), bind(a.rejected));
                if u::call(g, u::get(g, p, "then"), p, &[ok, err]).is_err() {
                    on_update(g, seq, Ok(()));
                }
            }
            Err(e) => on_update(g, seq, Err(e)),
        }
    }
}

unsafe fn handle(g: *mut c_void, note: Note) {
    unsafe {
        match note {
            Note::Update(seq) => call_update(g, seq),
            Note::Ready(seq) => {
                if let Some(s) =
                    PENDING.with(|p| p.borrow_mut().get_mut(&seq).and_then(|r| r.ready.take()))
                {
                    s.resolve(g, u::UNDEF);
                }
            }
            Note::Skipped(seq, reason) => {
                let taken = PENDING.with(|p| {
                    let mut p = p.borrow_mut();
                    let r = p.get_mut(&seq)?;
                    r.ended = true;
                    Some((r.ready.take(), r.outcome))
                });
                if let Some((Some(s), outcome)) = taken {
                    let value = match (&reason, outcome) {
                        (Reason::Rejected, Some(Err(cause))) => cause,
                        _ => {
                            let (name, message) = reason.error();
                            error(g, name, &message)
                        }
                    };
                    s.reject(g, value);
                }
                try_finish(g, seq);
            }
            Note::Finished(seq) => {
                PENDING.with(|p| {
                    if let Some(r) = p.borrow_mut().get_mut(&seq) {
                        r.ended = true;
                    }
                });
                try_finish(g, seq);
            }
        }
    }
}

unsafe fn handle_all(g: *mut c_void) {
    unsafe {
        loop {
            let notes = vt::take_notes();
            if notes.is_empty() {
                break;
            }
            for n in notes {
                handle(g, n);
            }
        }
    }
}

/// Settles what the transitions decided since the last frame and runs owed update callbacks.
/// `true` when anything ran.
pub(crate) fn deliver(window: &mut gpui::Window, cx: &mut gpui::App) -> bool {
    if !vt::has_notes() {
        return false;
    }
    if PENDING.with(|p| p.borrow().is_empty()) {
        vt::take_notes();
        return false;
    }
    crate::render::js_wave_app(window, cx, || {
        sk_runtime::bridge::settle_owed_drain();
        JsRuntime::with(|rt| {
            unsafe { handle_all(rt.raw()) };
            rt.ctx().drain_microtasks();
        });
        true
    });
    true
}

pub(crate) fn install(rt: &JsRuntime) {
    unsafe {
        let g = rt.raw();
        let class = u::define_class(
            g,
            &u::ClassSpec {
                name: "ViewTransition",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[("skipTransition", 0, skip_transition)],
                getters: &[
                    ("updateCallbackDone", update_callback_done),
                    ("ready", ready),
                    ("finished", finished),
                    ("types", types),
                ],
                accessors: &[],
            },
        );
        let global = glue::rjs_global(g);
        let promise = u::get(g, global, "Promise");
        let array = u::get(g, global, "Array");
        let fns = [
            u::make_fn(g, "", 1, update_fulfilled),
            u::make_fn(g, "", 2, update_rejected),
            u::make_fn(g, "", 0, noop),
        ];
        let (promise_resolve, set, array_from) = (
            u::get(g, promise, "resolve"),
            u::get(g, global, "Set"),
            u::get(g, array, "from"),
        );
        for v in [
            class.proto,
            promise,
            promise_resolve,
            set,
            array_from,
            fns[0],
            fns[1],
            fns[2],
        ] {
            redjs_protect(g, v);
        }
        API.with(|a| {
            a.set(Api {
                proto: class.proto,
                bag: u::atom_priv(g, "sk#viewTransition") as usize,
                promise,
                promise_resolve,
                set,
                array_from,
                fulfilled: fns[0],
                rejected: fns[1],
                noop: fns[2],
            })
        });
        rt.register("__sk_view_transitions_attach", 1, attach as *const c_void);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn js(rt: &JsRuntime, source: &str) {
        if let Err(e) = rt.eval(&format!("(()=>{{{source}\n}})()"), "view-transition-test") {
            panic!("{}\n{}", e.header, e.stack);
        }
    }

    fn settle(rt: &JsRuntime) {
        unsafe { handle_all(rt.raw()) };
        rt.ctx().drain_microtasks();
    }

    fn log(rt: &JsRuntime) -> String {
        js(
            rt,
            "globalThis.vtOut = vtLog.sort().join(); vtLog.length = 0;",
        );
        let out = rt.ctx().global_get("vtOut").unwrap();
        unsafe { u::read_string(rt.raw(), out.0) }.unwrap_or_default()
    }

    fn setup(rt: &JsRuntime) {
        install(rt);
        vt::reset();
        PENDING.with(|p| p.borrow_mut().clear());
        js(
            rt,
            "globalThis.vtDoc = { hidden: false }; __sk_view_transitions_attach(vtDoc);
             globalThis.vtLog = [];
             globalThis.track = (t, tag) => {
               for (const k of ['updateCallbackDone', 'ready', 'finished'])
                 t[k].then(() => vtLog.push(tag + '.' + k), (e) => vtLog.push(tag + '.' + k + ':' + (e && e.name)));
             };",
        );
    }

    #[test]
    fn a_skipped_transition_still_runs_its_update_and_finishes() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    "const a = vtDoc.startViewTransition(() => vtLog.push('a.cb')); track(a, 'a');
                     vtLog.push('active:' + (vtDoc.activeViewTransition === a));
                     const b = vtDoc.startViewTransition({ update: () => vtLog.push('b.cb'), types: ['x', 'y'] }); track(b, 'b');
                     vtLog.push('types:' + [...b.types].join('+'), 'instance:' + (b instanceof ViewTransition));
                     b.skipTransition();
                     vtLog.push('cleared:' + (vtDoc.activeViewTransition === null));",
                );
                settle(rt);
                assert_eq!(
                    log(rt),
                    "a.cb,a.finished,a.ready:AbortError,a.updateCallbackDone,active:true,\
                     b.cb,b.finished,b.ready:AbortError,b.updateCallbackDone,cleared:true,\
                     instance:true,types:x+y"
                );
            })
        });
    }

    #[test]
    fn ready_and_finished_follow_the_update_and_the_animations() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    "globalThis.t = vtDoc.startViewTransition(() => new Promise((r) => (globalThis.done = r)));
                     track(t, 't');",
                );
                vt::step();
                settle(rt);
                assert_eq!(log(rt), "", "nothing settles while the update runs");
                js(rt, "done();");
                rt.ctx().drain_microtasks();
                assert_eq!(log(rt), "t.updateCallbackDone");
                vt::step();
                settle(rt);
                assert_eq!(log(rt), "t.ready");
                vt::step();
                settle(rt);
                assert_eq!(log(rt), "t.finished");
                js(rt, "vtLog.push(String(vtDoc.activeViewTransition));");
                assert_eq!(log(rt), "null");
            })
        });
    }

    #[test]
    fn a_failed_update_rejects_everything_with_its_reason() {
        sk_runtime::testing::on_js_thread(|| {
            JsRuntime::with(|rt| {
                setup(rt);
                js(
                    rt,
                    "const t = vtDoc.startViewTransition(() => { throw new RangeError('boom'); }); track(t, 't');
                     vtDoc.hidden = true;
                     const h = vtDoc.startViewTransition(); track(h, 'h');
                     vtLog.push('h.active:' + (vtDoc.activeViewTransition === h));
                     try { new ViewTransition(); } catch (e) { vtLog.push('ctor:' + e.name); }
                     try { vtDoc.startViewTransition({ update: 1 }); } catch (e) { vtLog.push('option:' + e.name); }",
                );
                settle(rt);
                assert_eq!(
                    log(rt),
                    "ctor:TypeError,h.active:false,h.finished,h.ready:InvalidStateError,\
                     h.updateCallbackDone,option:TypeError,t.finished:RangeError,\
                     t.ready:AbortError,t.updateCallbackDone:RangeError"
                );
                js(
                    rt,
                    "vtDoc.hidden = false;
                     const t = vtDoc.startViewTransition(() => { throw new RangeError('boom'); }); track(t, 't');",
                );
                vt::step();
                settle(rt);
                assert_eq!(
                    log(rt),
                    "t.finished:RangeError,t.ready:RangeError,t.updateCallbackDone:RangeError"
                );
            })
        });
    }
}
