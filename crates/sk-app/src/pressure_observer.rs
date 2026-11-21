use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::hash::{BuildHasher, RandomState};
use std::sync::OnceLock;

use sk_css::ComputePressure;
use sk_runtime::redjs::capi::JSClassRef;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::redjs::{glue, webapi::events};
use sk_runtime::{host, host_fn, JsRuntime};

const MAX_QUEUED: usize = 10;

#[derive(Clone, Copy)]
pub(crate) struct Sample {
    pub state: ComputePressure,
    pub time: f64,
}

#[derive(Default)]
struct Observer {
    interval: Option<u32>,
    last: Option<Sample>,
    records: VecDeque<Sample>,
    rate: RateLimit,
}

struct RateLimit {
    window_ms: f64,
    threshold: usize,
    penalty_ms: f64,
    recent: VecDeque<f64>,
    delayed: Option<(f64, Sample)>,
}

impl Default for RateLimit {
    fn default() -> Self {
        let random = RandomState::new();
        Self {
            window_ms: (300_000 + random.hash_one(0) % 300_001) as f64,
            threshold: (50 + random.hash_one(1) % 51) as usize,
            penalty_ms: (5_000 + random.hash_one(2) % 5_001) as f64,
            recent: VecDeque::new(),
            delayed: None,
        }
    }
}

impl Observer {
    fn queue(&mut self, sample: Sample, now: f64) {
        let Some(interval) = self.interval else {
            return;
        };
        if let Some((until, delayed)) = self.rate.delayed.as_mut() {
            if sample.time > delayed.time {
                *delayed = sample;
            }
            if now >= *until {
                let latest = *delayed;
                self.rate.delayed = None;
                self.push(latest);
            }
            return;
        }
        if self.last.is_some_and(|p| {
            sample.time < p.time + f64::from(interval) || (interval == 0 && sample.state == p.state)
        }) {
            return;
        }
        while self
            .rate
            .recent
            .front()
            .is_some_and(|time| *time <= now - self.rate.window_ms)
        {
            self.rate.recent.pop_front();
        }
        if self.rate.recent.len() >= self.rate.threshold {
            self.rate.recent.clear();
            self.rate.delayed = Some((now + self.rate.penalty_ms, sample));
            sk_runtime::frames::wake();
            return;
        }
        self.rate.recent.push_back(now);
        self.push(sample);
    }

    fn push(&mut self, sample: Sample) {
        if self.records.len() >= MAX_QUEUED {
            self.records.pop_front();
        }
        self.records.push_back(sample);
        self.last = Some(sample);
    }
}

thread_local! {
    static CALLBACK: Cell<usize> = const { Cell::new(0) };
    static OBSERVER_PROTO: Cell<u64> = const { Cell::new(0) };
    static RECORD_PROTO: Cell<u64> = const { Cell::new(0) };
    static REPORT_ERROR: Cell<u64> = const { Cell::new(0) };
    static KNOWN_SOURCES: Cell<u64> = const { Cell::new(0) };
    static ACTIVE: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

unsafe fn observer_class() -> JSClassRef {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe { make_host_class::<Observer>("PressureObserver") as usize })
        as JSClassRef
}

unsafe fn record_class() -> JSClassRef {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe { make_host_class::<Sample>("PressureRecord") as usize })
        as JSClassRef
}

unsafe fn observer(g: *mut c_void, receiver: u64) -> Option<&'static mut Observer> {
    unsafe { host_data(g, observer_class(), receiver) }
}

unsafe fn record(g: *mut c_void, receiver: u64) -> Option<Sample> {
    unsafe { host_data::<Sample>(g, record_class(), receiver).copied() }
}

host_fn!(fn construct(g, args) {
    unsafe {
        let callback = args.arg(0).0;
        if !events::is_callable(g, callback) {
            return throw_type_error(g, "PressureObserver requires a callback function");
        }
        let Ok(proto) = try_get(g, args.this().0, "prototype") else { return UNDEF };
        let proto = if glue::rjs_kind(g, proto) >= 8 {
            proto
        } else {
            OBSERVER_PROTO.with(Cell::get)
        };
        let object = host_create(g, observer_class(), proto, Observer::default());
        glue::rjs_define(g, object, CALLBACK.with(Cell::get) as *mut c_void, callback, 1 | 2 | 4);
        object
    }
});

unsafe fn source(g: *mut c_void, v: u64) -> Result<bool, u64> {
    let mut text = UNDEF;
    if unsafe { glue::rjs_to_string(g, v, &mut text) } != 0 {
        return Err(text);
    }
    Ok(unsafe { read_string(g, text) }.as_deref() == Some("cpu"))
}

unsafe fn sample_interval(g: *mut c_void, options: u64) -> Result<u32, u64> {
    unsafe {
        if options == UNDEF || options == NULL {
            return Ok(0);
        }
        if glue::rjs_kind(g, options) < 8 {
            return Err(host::error(
                g,
                "TypeError",
                "Expected PressureObserver options",
            ));
        }
        let key = atom(g, "sampleInterval");
        let mut value = UNDEF;
        let status = glue::rjs_get(g, options, key, &mut value);
        glue::rjs_atom_free(g, key);
        if status != 0 {
            return Err(value);
        }
        if value == UNDEF {
            return Ok(0);
        }
        let mut interval = 0.;
        let mut error = UNDEF;
        if glue::rjs_to_number_x(g, value, &mut interval, &mut error) != 0 {
            return Err(error);
        }
        interval = interval.trunc();
        if !(0. ..=f64::from(u32::MAX)).contains(&interval) {
            return Err(host::error(
                g,
                "TypeError",
                "sampleInterval must be an unsigned long",
            ));
        }
        Ok(interval as u32)
    }
}

host_fn!(fn observe(g, args) {
    unsafe {
        let object = args.this().0;
        if observer(g, object).is_none() {
            return rejected_promise(g, 1, "Illegal invocation");
        }
        match source(g, args.arg(0).0) {
            Err(error) => return host::rejected_with(g, error),
            Ok(false) => return rejected_promise(g, 1, "Expected the pressure source 'cpu'"),
            Ok(true) => {}
        }
        let interval = match sample_interval(g, args.arg(1).0) {
            Err(error) => return host::rejected_with(g, error),
            Ok(interval) => interval,
        };
        let observer = observer(g, object).expect("branded before converting arguments");
        if observer.interval.replace(interval).is_none() {
            redjs_protect(g, object);
            ACTIVE.with(|list| list.borrow_mut().push(object));
        }
        sk_runtime::frames::wake();
        resolved_promise(g, UNDEF)
    }
});

unsafe fn remove(g: *mut c_void, object: u64) {
    if let Some(observer) = unsafe { observer(g, object) } {
        *observer = Observer::default();
        let removed = ACTIVE.with(|list| {
            let mut list = list.borrow_mut();
            let Some(index) = list.iter().position(|&v| v == object) else {
                return false;
            };
            list.remove(index);
            true
        });
        if removed {
            unsafe { redjs_unprotect(g, object) };
            sk_runtime::frames::wake();
        }
    }
}

host_fn!(fn unobserve(g, args) {
    unsafe {
        let object = args.this().0;
        if observer(g, object).is_none() {
            return throw_type_error(g, "Illegal invocation");
        }
        match source(g, args.arg(0).0) {
            Ok(true) => remove(g, object),
            Ok(false) => return throw_type_error(g, "Expected the pressure source 'cpu'"),
            Err(error) => glue::rjs_throw(g, error),
        }
        UNDEF
    }
});

host_fn!(fn disconnect(g, args) {
    unsafe {
        let object = args.this().0;
        if observer(g, object).is_none() {
            return throw_type_error(g, "Illegal invocation");
        }
        remove(g, object);
        UNDEF
    }
});

unsafe fn make_records(g: *mut c_void, records: VecDeque<Sample>) -> u64 {
    unsafe {
        let array = new_array(g);
        redjs_protect(g, array);
        let proto = RECORD_PROTO.with(Cell::get);
        for (index, sample) in records.into_iter().enumerate() {
            let elem = host_create(g, record_class(), proto, sample);
            set_i(g, array, index as u32, elem);
        }
        redjs_unprotect(g, array);
        array
    }
}

host_fn!(fn take_records(g, args) {
    unsafe {
        let Some(observer) = observer(g, args.this().0) else {
            return throw_type_error(g, "Illegal invocation");
        };
        make_records(g, std::mem::take(&mut observer.records))
    }
});

host_fn!(fn known_sources(_g, _args) { KNOWN_SOURCES.with(Cell::get) });

host_fn!(fn record_source(g, args) {
    unsafe {
        if record(g, args.this().0).is_none() { return throw_type_error(g, "Illegal invocation"); }
        js_string(g, "cpu")
    }
});

host_fn!(fn record_state(g, args) {
    unsafe {
        let Some(sample) = record(g, args.this().0) else { return throw_type_error(g, "Illegal invocation") };
        js_string(g, sample.state.as_str())
    }
});

host_fn!(fn record_time(g, args) {
    unsafe {
        let Some(sample) = record(g, args.this().0) else { return throw_type_error(g, "Illegal invocation") };
        enc_f64(sample.time)
    }
});

host_fn!(fn record_to_json(g, args) {
    unsafe {
        let Some(sample) = record(g, args.this().0) else { return throw_type_error(g, "Illegal invocation") };
        let obj = new_object(g, UNDEF);
        define_many(g, obj, &["source", "state", "time"], &[js_string(g, "cpu"), js_string(g, sample.state.as_str()), enc_f64(sample.time)], 0);
        obj
    }
});

pub(crate) fn install(rt: &JsRuntime) {
    let g = rt.raw();
    // SAFETY: class definitions and their rooted values belong to the host's live JS realm.
    unsafe {
        let global = glue::rjs_global(g);
        let report_error = get(g, global, "reportError");
        redjs_protect(g, report_error);
        REPORT_ERROR.with(|slot| slot.set(report_error));
        CALLBACK.with(|slot| slot.set(atom_priv(g, "PressureObserver callback") as usize));
        let record = define_class(
            g,
            &ClassSpec {
                name: "PressureRecord",
                arity: 0,
                construct: None,
                parent_proto: 0,
                methods: &[("toJSON", 0, record_to_json)],
                getters: &[
                    ("source", record_source),
                    ("state", record_state),
                    ("time", record_time),
                ],
                accessors: &[],
            },
        );
        redjs_protect(g, record.proto);
        RECORD_PROTO.with(|slot| slot.set(record.proto));
        let observer = define_class(
            g,
            &ClassSpec {
                name: "PressureObserver",
                arity: 1,
                construct: Some(construct),
                parent_proto: 0,
                methods: &[
                    ("observe", 1, observe),
                    ("unobserve", 1, unobserve),
                    ("disconnect", 0, disconnect),
                    ("takeRecords", 0, take_records),
                ],
                getters: &[],
                accessors: &[],
            },
        );
        redjs_protect(g, observer.proto);
        OBSERVER_PROTO.with(|slot| slot.set(observer.proto));
        let sources = new_array(g);
        redjs_protect(g, sources);
        set_i(g, sources, 0, js_string(g, "cpu"));
        let object = get(g, global, "Object");
        let _ = call(g, get(g, object, "freeze"), object, &[sources]);
        KNOWN_SOURCES.with(|slot| slot.set(sources));
        let key = atom(g, "knownSources");
        glue::rjs_define_accessor(
            g,
            observer.ctor,
            key,
            make_fn(g, "get knownSources", 0, known_sources),
            UNDEF,
            2,
        );
        glue::rjs_atom_free(g, key);
        define(g, global, "PressureRecord", record.ctor, 2);
        define(g, global, "PressureObserver", observer.ctor, 2);
    }
}

pub(crate) fn demand(rt: &JsRuntime) -> (bool, bool) {
    ACTIVE.with(|active| {
        let active = active.borrow();
        let repeat = active.iter().any(|&object| {
            unsafe { observer(rt.raw(), object) }.is_some_and(|o| {
                o.interval.is_some_and(|interval| interval > 0) || o.rate.delayed.is_some()
            })
        });
        (!active.is_empty(), repeat)
    })
}

pub(crate) fn collect(rt: &JsRuntime, sample: Sample) {
    let now = sk_runtime::redjs::web::now_ms();
    ACTIVE.with(|active| {
        for &object in active.borrow().iter() {
            if let Some(observer) = unsafe { observer(rt.raw(), object) } {
                observer.queue(sample, now);
            }
        }
    });
}

pub(crate) fn deliver(rt: &JsRuntime, visible: bool) {
    let g = rt.raw();
    let pending = ACTIVE.with(|active| {
        active
            .borrow()
            .iter()
            .copied()
            .filter(|&object| unsafe { observer(g, object) }.is_some_and(|o| !o.records.is_empty()))
            .collect::<Vec<_>>()
    });
    // SAFETY: the additional roots survive a callback disconnecting a later observer.
    unsafe {
        for &object in &pending {
            redjs_protect(g, object);
        }
        for object in pending {
            let records = observer(g, object)
                .map(|o| std::mem::take(&mut o.records))
                .unwrap_or_default();
            if visible && !crate::window::page_hidden() && !records.is_empty() {
                let array = make_records(g, records);
                let callback = get_a(g, object, CALLBACK.with(Cell::get) as *mut c_void);
                if let Err(error) = call(g, callback, UNDEF, &[array, object]) {
                    let _ = call(
                        g,
                        REPORT_ERROR.with(Cell::get),
                        glue::rjs_global(g),
                        &[error],
                    );
                }
            }
            redjs_unprotect(g, object);
        }
    }
}

pub(crate) fn clear(rt: &JsRuntime) {
    let active = ACTIVE.with(|active| active.borrow().clone());
    for object in active {
        // SAFETY: each object has a root from observe and belongs to rt.
        unsafe { remove(rt.raw(), object) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changes_by_default_and_periodic_observers_obey_the_interval() {
        let mut observer = Observer {
            interval: Some(0),
            ..Observer::default()
        };
        let nominal = |time| Sample {
            state: ComputePressure::Nominal,
            time,
        };
        observer.queue(nominal(1000.), 1000.);
        observer.queue(nominal(2000.), 2000.);
        assert_eq!(observer.records.len(), 1);
        observer.interval = Some(3000);
        observer.queue(nominal(3000.), 3000.);
        assert_eq!(observer.records.len(), 1);
        observer.queue(nominal(4000.), 4000.);
        assert_eq!(observer.records.len(), 2);
        observer.interval = Some(1);
        for time in 4001..4049 {
            observer.queue(nominal(f64::from(time)), f64::from(time));
        }
        assert_eq!(observer.records.len(), MAX_QUEUED);
        assert_eq!(observer.records.pop_front().unwrap().time, 4039.);
    }

    #[test]
    fn exceeding_the_reporting_budget_defers_and_coalesces_records() {
        let mut observer = Observer {
            interval: Some(1),
            ..Observer::default()
        };
        observer.rate.threshold = 2;
        observer.rate.penalty_ms = 5000.;
        let fair = |time| Sample {
            state: ComputePressure::Fair,
            time,
        };
        for time in [1000., 2000., 3000., 4000.] {
            observer.queue(fair(time), time);
        }
        assert_eq!(observer.records.len(), 2);
        assert_eq!(observer.rate.delayed.unwrap().1.time, 4000.);
        observer.queue(fair(8000.), 8000.);
        assert!(observer.rate.delayed.is_none());
        assert_eq!(observer.records.pop_back().unwrap().time, 8000.);
    }
}
