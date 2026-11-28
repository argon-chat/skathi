mod native;
mod settings;
#[cfg(test)]
mod tests;
mod web;

use sk_gamepad::{DeadZones, Gamepads, Pad, PadEvent};
use sk_runtime::{frames, JsRuntime};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

const INPUT_INTERVAL: Duration = Duration::from_millis(8);
const DISCOVERY_INTERVAL: Duration = Duration::from_millis(250);
const IDLE_DRAIN_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub(super) struct Slot {
    pub index: u32,
    pub pad: Pad,
    pub timestamp: f64,
}

struct Sample {
    timestamp: f64,
    events: Vec<PadEvent>,
}

#[derive(Default)]
struct Service {
    backend: Option<Gamepads>,
    dead_zones: DeadZones,
    retry_at: Option<Instant>,
    slots: Rc<Vec<Slot>>,
    delivered: Rc<Vec<Slot>>,
    native_start: Option<(Rc<Vec<Slot>>, usize)>,
    samples: Vec<Sample>,
    last_sample: Option<Instant>,
    focused: bool,
}

thread_local! {
    static SERVICE: RefCell<Service> = RefCell::new(Service::default());
    static REQUESTED: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn activate() {
    if !REQUESTED.replace(true) {
        frames::wake();
    }
}

fn reconcile<'a>(
    old: &Rc<Vec<Slot>>,
    pads: impl ExactSizeIterator<Item = (u32, &'a Pad)> + Clone,
    events: &[PadEvent],
    now: f64,
) -> Rc<Vec<Slot>> {
    if events.is_empty()
        && pads.len() == old.len()
        && pads
            .clone()
            .all(|(i, p)| old.iter().any(|s| s.index == i && &s.pad == p))
    {
        return old.clone();
    }
    let mut slots: Vec<_> = pads
        .map(|(index, pad)| {
            let unchanged = old.iter().find(|s| s.index == index && &s.pad == pad);
            let timestamp = unchanged
                .filter(|_| !events.iter().any(|e| e.instance() == pad.instance))
                .map_or(now, |s| s.timestamp);
            Slot {
                index,
                pad: pad.clone(),
                timestamp,
            }
        })
        .collect();
    slots.sort_by_key(|s| s.index);
    Rc::new(slots)
}

impl Service {
    // WGI queues input on its own thread even while the host is parked. Drain that queue
    // privately in idle waves, keeping connection identities until the devices disappear.
    fn maintain_idle(&mut self, now: Instant) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        if self
            .last_sample
            .is_none_or(|last| now.duration_since(last) >= IDLE_DRAIN_INTERVAL)
        {
            let empty = backend.sample().len() == 0;
            self.dead_zones
                .connections
                .retain(|id, _| backend.pads().any(|(_, p)| p.instance == *id));
            // WGI stops its polling thread on drop. The Unix device monitors live for the process.
            if empty && cfg!(target_os = "windows") {
                self.backend = None;
                return;
            }
            self.last_sample = Some(now);
        }
        frames::wake_at(self.last_sample.unwrap_or(now) + IDLE_DRAIN_INTERVAL);
    }

    fn sample(&mut self, now: Instant) {
        if self
            .last_sample
            .is_some_and(|t| now.duration_since(t) < INPUT_INTERVAL)
        {
            return;
        }
        self.last_sample = Some(now);
        let _profile = sk_runtime::redjs::profiling::span("native", "Sample gamepads");
        if self.backend.is_none() {
            if self.retry_at.is_some_and(|t| t > now) {
                return;
            }
            match Gamepads::new() {
                Ok(mut backend) => {
                    backend.set_dead_zones(self.dead_zones.clone());
                    self.backend = Some(backend);
                }
                Err(error) => {
                    if self.retry_at.is_none() {
                        log::warn!("Gamepad initialization failed: {error}");
                    }
                    self.retry_at = Some(now + Duration::from_secs(5));
                    return;
                }
            }
            self.retry_at = None;
        }
        let backend = self.backend.as_mut().unwrap();
        let _ = backend.sample();
        let events = backend.take_events();
        if !self.dead_zones.connections.is_empty() {
            for event in &events {
                if let PadEvent::Disconnected(pad) = event {
                    self.dead_zones.connections.remove(&pad.instance);
                }
            }
        }
        let timestamp = sk_runtime::redjs::web::now_ms();
        self.slots = reconcile(&self.slots, backend.pads(), &events, timestamp);
        if !events.is_empty() {
            self.samples.push(Sample { timestamp, events });
        }
    }
}

fn snapshot() -> Rc<Vec<Slot>> {
    activate();
    SERVICE.with_borrow_mut(|s| {
        if s.focused {
            s.sample(Instant::now());
        }
        s.slots.clone()
    })
}

// Input accumulated while a controller was idle predates the first native subscription.
// Keep it available to web gesture detection and start the native stream at this position.
fn start_native_stream() {
    SERVICE.with_borrow_mut(|s| {
        if s.focused {
            s.last_sample = None;
            s.sample(Instant::now());
        }
        s.native_start = Some((s.slots.clone(), s.samples.len()));
    });
}

pub(super) fn rumble(instance: u32, strong: f64, weak: f64, duration_ms: u32) -> bool {
    SERVICE.with_borrow_mut(|s| {
        if duration_ms > 0 && !s.focused {
            return false;
        }
        s.backend
            .as_mut()
            .is_some_and(|b| b.rumble(instance, strong, weak, duration_ms))
    })
}

pub(crate) fn pump(rt: &JsRuntime, focused: bool, document_visible: bool) {
    let requested = REQUESTED.replace(false);
    let web_focused = focused && document_visible;
    let listeners = native::observed() || (web_focused && web::observed(rt));
    let pending = web::pending();
    let now = Instant::now();
    let update = SERVICE.with_borrow_mut(|s| {
        let was_focused = s.focused;
        s.focused = focused;
        if !focused && was_focused {
            if let Some(backend) = s.backend.as_mut() {
                for slot in s.slots.iter().filter(|slot| slot.pad.can_rumble) {
                    backend.rumble(slot.pad.instance, 0.0, 0.0, 0);
                }
            }
        }
        let resuming = !was_focused && s.backend.is_some();
        if focused && (requested || listeners || pending || resuming) {
            if !was_focused {
                s.last_sample = None;
            }
            s.sample(now);
            if listeners || pending {
                let interval = if s.slots.is_empty() {
                    DISCOVERY_INTERVAL
                } else {
                    INPUT_INTERVAL
                };
                frames::wake_at(s.retry_at.unwrap_or(now + interval));
            } else if s.backend.is_some() {
                frames::wake_at(now + IDLE_DRAIN_INTERVAL);
            }
        } else {
            s.maintain_idle(now);
        }
        let changed = !Rc::ptr_eq(&s.slots, &s.delivered);
        if changed
            || was_focused != focused
            || web::focus_changed(web_focused)
            || requested
            || !s.samples.is_empty()
        {
            let slots = s.slots.clone();
            let previous = std::mem::replace(&mut s.delivered, slots.clone());
            Some((
                previous,
                slots,
                was_focused,
                std::mem::take(&mut s.samples),
                s.native_start.take(),
            ))
        } else {
            None
        }
    });
    if update.is_none() && !pending {
        return;
    }
    let _profile = sk_runtime::redjs::profiling::span("native", "Deliver gamepad input");
    // One JS input turn per host wave. Releasing JSC's outermost API lock between
    // individual properties or callbacks would repeatedly drain microtasks mid-batch.
    rt.locked(|| {
        if let Some((previous, slots, was_focused, samples, native_start)) = update {
            let observers = native::listeners();
            web::update(rt, &slots, web_focused, &samples);
            if focused && !observers.is_empty() {
                let (baseline, offset) = native_start.unwrap_or((previous, 0));
                native::deliver(
                    rt.ctx(),
                    &observers,
                    &baseline,
                    &slots,
                    was_focused,
                    &samples[offset..],
                );
            }
        }
        if pending {
            web::tick(rt);
        }
    });
}

pub(crate) fn install(rt: &JsRuntime) -> Result<(), String> {
    native::install(rt.ctx()).map_err(|e| e.to_string())?;
    web::install(rt);
    crate::js::on_reload(|ctx| clear(ctx, true));
    crate::js::on_shutdown(|ctx| clear(ctx, false));
    Ok(())
}

fn clear(ctx: &crate::js::Context, reload: bool) {
    native::clear(ctx);
    JsRuntime::with(web::shutdown);
    SERVICE.with_borrow_mut(|s| {
        let mut backend = if reload && cfg!(unix) {
            s.backend.take()
        } else {
            None
        };
        if let Some(backend) = &mut backend {
            backend.set_dead_zones(DeadZones::default());
            backend.reset_session();
        }
        *s = Service {
            backend,
            ..Default::default()
        };
    });
    REQUESTED.set(false);
}
