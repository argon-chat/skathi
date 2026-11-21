use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{App, Window};
use sk_css::{ComputePressure, PowerScheme};
use sk_desktop::power::{self as system, Pressure, Scheme};
use sk_runtime::JsRuntime;

use crate::pressure_observer::{self, Sample};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PowerState {
    pub(crate) scheme: PowerScheme,
    pub(crate) pressure: ComputePressure,
    pub(crate) on_battery: bool,
}

#[derive(Clone, Copy, Default)]
struct Emulation {
    scheme: Option<PowerScheme>,
    pressure: Option<ComputePressure>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct WindowUse {
    visible: bool,
    quality: bool,
}

thread_local! {
    static STATE: Cell<PowerState> = Cell::new(PowerState::default());
    static EMULATION: Cell<Emulation> = const { Cell::new(Emulation { scheme: None, pressure: None }) };
    static VIRTUAL_SAMPLE: Cell<Option<(Instant, Sample)>> = const { Cell::new(None) };
    static NATIVE_START: Cell<Option<u64>> = const { Cell::new(None) };
    static LAST_NATIVE_PRESSURE: Cell<ComputePressure> = const { Cell::new(ComputePressure::Nominal) };
    static MATCH_MEDIA_DEMAND: Cell<bool> = const { Cell::new(false) };
    static MAIN_QUALITY: Cell<bool> = const { Cell::new(false) };
    static POPOUTS: RefCell<HashMap<u64, WindowUse>> = RefCell::new(HashMap::new());
    #[cfg(feature = "battery")]
    static BATTERY_MONITOR: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn install(rt: &JsRuntime) {
    system::init();
    let initial = PowerState {
        scheme: native_scheme(),
        ..PowerState::default()
    };
    STATE.with(|state| state.set(initial));
    gpui::set_auto_effect_quality(initial.effect_quality());
    pressure_observer::install(rt);
}

pub(crate) fn state() -> PowerState {
    STATE.with(Cell::get)
}

pub(crate) fn set_match_media_demand(demand: bool) {
    if MATCH_MEDIA_DEMAND.with(|current| current.replace(demand)) != demand {
        sk_runtime::frames::wake();
    }
}

pub(crate) fn note_popout(window: &Window) {
    let id = window.window_handle().window_id().as_u64();
    let visible = window.vn_is_shown() && !window.vn_is_minimized();
    let status = WindowUse {
        visible,
        quality: visible && window.vn_uses_auto_effect_quality(),
    };
    let old = POPOUTS.with(|all| all.borrow_mut().insert(id, status));
    if old != Some(status) {
        sk_runtime::frames::wake();
    }
}

pub(crate) fn note_primary(window: &Window) {
    let quality = window.vn_uses_auto_effect_quality();
    if MAIN_QUALITY.with(|old| old.replace(quality)) != quality {
        sk_runtime::frames::wake();
    }
}

pub(crate) fn forget_popout(id: u64) {
    if POPOUTS.with(|all| all.borrow_mut().remove(&id)).is_some() {
        sk_runtime::frames::wake();
    }
}

fn native_scheme() -> PowerScheme {
    match system::scheme() {
        Scheme::Balanced => PowerScheme::Balanced,
        Scheme::Saving => PowerScheme::Saving,
        Scheme::Performance => PowerScheme::Performance,
    }
}

fn compute_pressure(pressure: Pressure) -> ComputePressure {
    match pressure {
        Pressure::Nominal => ComputePressure::Nominal,
        Pressure::Fair => ComputePressure::Fair,
        Pressure::Serious => ComputePressure::Serious,
        Pressure::Critical => ComputePressure::Critical,
    }
}

fn native_sample(demand: bool, repeat: bool) -> Option<Sample> {
    if !demand {
        if NATIVE_START.with(Cell::take).is_some() {
            system::set_pressure_demand(false, false);
        }
        return None;
    }
    let sample = system::pressure_sample();
    let ready = NATIVE_START.with(|start| match start.get() {
        Some(serial) => sample.serial != serial,
        None => {
            start.set(Some(sample.serial));
            false
        }
    });
    system::set_pressure_demand(true, repeat);
    ready.then(|| {
        let state = compute_pressure(sample.state);
        LAST_NATIVE_PRESSURE.with(|last| last.set(state));
        Sample {
            state,
            time: sample.time,
        }
    })
}

fn emulated_sample(state: ComputePressure, repeat: bool) -> Sample {
    const PERIOD: Duration = Duration::from_secs(1);
    let now = Instant::now();
    VIRTUAL_SAMPLE.with(|slot| {
        let (at, sample) = slot
            .get()
            .filter(|(at, sample)| {
                sample.state == state && (!repeat || now.duration_since(*at) < PERIOD)
            })
            .unwrap_or_else(|| {
                let next = Sample {
                    state,
                    time: sk_runtime::redjs::web::now_ms(),
                };
                slot.set(Some((now, next)));
                (now, next)
            });
        if repeat {
            sk_runtime::frames::wake_at(at + PERIOD);
        }
        sample
    })
}

fn on_battery(rt: &JsRuntime, quality: bool) -> bool {
    #[cfg(feature = "battery")]
    {
        use sk_runtime::redjs::webapi::battery;
        let g = rt.raw();
        // SAFETY: rt is the installed battery provider's realm on its owning thread.
        unsafe {
            if quality && !BATTERY_MONITOR.with(|active| active.replace(true)) {
                battery::monitor(g);
            }
            battery::on_battery(g)
        }
    }
    #[cfg(not(feature = "battery"))]
    {
        let _ = (rt, quality);
        false
    }
}

pub(crate) fn refresh(rt: &JsRuntime, window: &mut Window, cx: &mut App, visible: bool) {
    let (other_visible, other_quality) = POPOUTS.with(|all| {
        let all = all.borrow();
        (
            all.values().any(|w| w.visible),
            all.values().any(|w| w.quality),
        )
    });
    let quality = other_quality || (visible && window.vn_uses_auto_effect_quality());
    let may_observe = visible && (crate::window::app_focused(cx) || crate::quiet::never_focus());
    let (observer, repeat) = if may_observe {
        pressure_observer::demand(rt)
    } else {
        (false, false)
    };
    let css = (visible || other_visible) && sk_tree::uses_compute_pressure();
    let query = visible && MATCH_MEDIA_DEMAND.with(Cell::get);
    let emulation = EMULATION.with(Cell::get);
    let real = emulation.pressure.is_none() && (quality || observer || css || query);
    let sample = native_sample(real, real && repeat);
    let sample = if let Some(state) = emulation.pressure {
        Some(emulated_sample(state, repeat))
    } else {
        VIRTUAL_SAMPLE.with(|slot| slot.set(None));
        sample
    };
    let previous = state();
    let next = PowerState {
        scheme: emulation.scheme.unwrap_or_else(native_scheme),
        pressure: sample.map_or_else(|| LAST_NATIVE_PRESSURE.with(Cell::get), |s| s.state),
        on_battery: on_battery(rt, quality),
    };
    if next != previous {
        STATE.with(|state| state.set(next));
        let quality = next.effect_quality();
        if quality != previous.effect_quality() {
            gpui::set_auto_effect_quality(quality);
            // Discard retained child paint operations containing the previous tier.
            window.refresh();
        }
        crate::notify_shell(cx);
        crate::popout::notify_all(cx);
    }
    if observer {
        if let Some(sample) = sample {
            pressure_observer::collect(rt, sample);
        }
    }
}

#[cfg(sk_devtools)]
pub(crate) fn command(
    scheme: Option<&str>,
    pressure: Option<&str>,
) -> Result<crate::json::Json, &'static str> {
    use crate::json::Json;

    let mut emulation = EMULATION.with(Cell::get);
    if let Some(value) = scheme {
        emulation.scheme = match value {
            "system" => None,
            _ => Some(
                PowerScheme::from_keyword(value)
                    .ok_or("scheme must be balanced, saving, performance or system")?,
            ),
        };
    }
    if let Some(value) = pressure {
        emulation.pressure = match value {
            "system" => None,
            _ => Some(
                ComputePressure::from_keyword(value)
                    .ok_or("pressure must be nominal, fair, serious, critical or system")?,
            ),
        };
    }
    EMULATION.with(|slot| slot.set(emulation));
    if scheme.is_some() || pressure.is_some() {
        sk_runtime::frames::wake();
    }
    let state = state();
    Ok(Json::obj(vec![
        (
            "scheme",
            Json::s(emulation.scheme.unwrap_or(state.scheme).as_str()),
        ),
        (
            "pressure",
            Json::s(emulation.pressure.unwrap_or(state.pressure).as_str()),
        ),
        ("onBattery", Json::Bool(state.on_battery)),
        (
            "schemeOverride",
            emulation.scheme.map_or(Json::Null, |v| Json::s(v.as_str())),
        ),
        (
            "pressureOverride",
            emulation
                .pressure
                .map_or(Json::Null, |v| Json::s(v.as_str())),
        ),
    ]))
}
