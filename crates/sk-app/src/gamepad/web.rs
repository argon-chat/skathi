//! The window's Gamepad API. Polls return snapshots, while the object delivered in a
//! connection event stays live until that particular device disconnects.

mod haptics;
mod objects;
#[cfg(test)]
mod tests;

use super::{Sample, Slot};
use sk_gamepad::PadEvent;
use sk_runtime::redjs::ffi::{redjs_protect, redjs_unprotect};
use sk_runtime::redjs::webapi::{events, util::*};
use sk_runtime::{host, host_fn, JsRuntime};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::{Arc, Once};

#[derive(Default)]
struct Page {
    focused: bool,
    exposed: bool,
    disconnecting: Option<u32>,
    devices: Vec<Device>,
}

struct Device {
    state: Arc<Slot>,
    event_pad: Option<u64>,
}

thread_local! {
    static PAGE: RefCell<Page> = RefCell::default();
    static MAY_LISTEN: Cell<bool> = const { Cell::new(false) };
    static SESSION: Cell<u64> = const { Cell::new(0) };
}

fn observe_listener(name: &str) {
    if matches!(name, "gamepadconnected" | "gamepaddisconnected") && objects::realm().navigator != 0
    {
        MAY_LISTEN.set(true);
        super::activate();
    }
}

pub(super) fn observed(rt: &JsRuntime) -> bool {
    if !MAY_LISTEN.get() {
        return false;
    }
    let present = rt.locked(|| unsafe {
        let g = rt.raw();
        let target = host::global(g);
        events::has_listener(g, target, "gamepadconnected")
            || events::has_listener(g, target, "gamepaddisconnected")
    });
    // Adding either listener arms the check again, including the ongamepad* setters.
    MAY_LISTEN.set(present);
    present
}

pub(super) fn install(rt: &JsRuntime) {
    static LISTEN: Once = Once::new();
    LISTEN.call_once(|| events::on_listener_added(observe_listener));
    objects::install(rt);
}

pub(super) fn focus_changed(focused: bool) -> bool {
    PAGE.with_borrow(|page| page.focused != focused)
}

host_fn!(fn get_gamepads(g, args) {
    unsafe {
        if args.this().0 != objects::realm().navigator {
            return throw_type_error(g, "Illegal invocation");
        }
        super::activate();
        let slots = PAGE.with_borrow(|page| {
            if !page.exposed || !page.focused { return Vec::new() }
            page.devices.iter().map(|device| {
                (device.state.clone(), page.disconnecting != Some(device.state.pad.instance))
            }).collect()
        });
        let result = new_array(g);
        redjs_protect(g, result);
        // Disconnected slots in the middle must be explicit nulls rather than array holes.
        let mut next_index = 0;
        for (slot, connected) in slots {
            let index = slot.index;
            while next_index < index {
                set_i(g, result, next_index, NULL);
                next_index += 1;
            }
            set_i(g, result, index, objects::pad(g, slot, connected));
            next_index = index + 1;
        }
        redjs_unprotect(g, result);
        result
    }
});

fn gesture(old: &Slot, new: &Slot, samples: &[Sample]) -> Option<f64> {
    let was_neutral = |i: usize| old.pad.buttons.get(i).is_none_or(|v| *v <= 0.5);
    let crossed = |before: f64, value: f64| before.abs() <= 0.5 && value.abs() > 0.5;
    let mut calibrated = old.pad.dead_zone != new.pad.dead_zone;
    for sample in samples {
        for event in sample
            .events
            .iter()
            .filter(|e| e.instance() == new.pad.instance)
        {
            match event {
                PadEvent::Button(b)
                    if was_neutral(b.index)
                        && b.index < new.pad.buttons.len()
                        && b.before <= 0.5
                        && b.value > 0.5 =>
                {
                    return Some(sample.timestamp)
                }
                PadEvent::DeadZone { .. }
                | PadEvent::Axis {
                    recalibrated: true, ..
                } => {
                    calibrated = true;
                }
                PadEvent::Axis {
                    index,
                    before,
                    value,
                    ..
                } if new.pad.standard
                    && *index < new.pad.axes.len()
                    && crossed(*before, *value) =>
                {
                    return Some(sample.timestamp);
                }
                _ => {}
            }
        }
    }
    // A page polling infrequently can miss the input journal while the host is idle.
    // Its final axis values are comparable only when the same calibration was in effect.
    let button = new
        .pad
        .buttons
        .iter()
        .enumerate()
        .any(|(i, value)| was_neutral(i) && *value > 0.5);
    let stick = new.pad.standard
        && !calibrated
        && old
            .pad
            .axes
            .iter()
            .zip(&new.pad.axes)
            .any(|(&a, &b)| crossed(a, b));
    (button || stick).then_some(new.timestamp)
}

fn remove_devices(g: *mut c_void, pads: &[Slot]) {
    let removed: Vec<_> = PAGE.with_borrow(|page| {
        page.devices
            .iter()
            .map(|d| d.state.pad.instance)
            .filter(|id| !pads.iter().any(|p| p.pad.instance == *id))
            .collect()
    });
    for id in removed {
        let object = PAGE.with_borrow_mut(|page| {
            page.disconnecting = Some(id);
            page.devices
                .iter()
                .find(|d| d.state.pad.instance == id)?
                .event_pad
        });
        haptics::preempt(g, id);
        if let Some(object) = object {
            unsafe {
                objects::set_connected(g, object, false);
                // Within the disconnect callback the slot still contains the disconnected Gamepad.
                objects::dispatch(g, "gamepaddisconnected", object);
            }
        }
        PAGE.with_borrow_mut(|page| {
            page.disconnecting = None;
            if let Some(index) = page.devices.iter().position(|d| d.state.pad.instance == id)
                && let Some(object) = page.devices.remove(index).event_pad
            {
                unsafe { redjs_unprotect(g, object) };
            }
        });
    }
}

pub(super) fn update(rt: &JsRuntime, pads: &[Slot], focused: bool, samples: &[Sample]) {
    let g = rt.raw();
    let previous_focus = PAGE.with_borrow_mut(|page| std::mem::replace(&mut page.focused, focused));
    if !focused {
        haptics::cancel_unavailable(g);
        return;
    }
    remove_devices(g, pads);
    let added = PAGE.with_borrow_mut(|page| {
        let first_gesture = if !page.exposed && previous_focus {
            pads.iter()
                .filter_map(|p| {
                    page.devices
                        .iter()
                        .find(|d| d.state.pad.instance == p.pad.instance)
                        .and_then(|d| gesture(&d.state, p, samples))
                })
                .min_by(f64::total_cmp)
        } else {
            None
        };
        page.exposed |= first_gesture.is_some();
        let mut added = Vec::new();
        for slot in pads {
            let snapshot = || {
                let mut slot = slot.clone();
                // Earlier samples are private until the first gamepad gesture in this document.
                if let Some(time) = first_gesture {
                    slot.timestamp = slot.timestamp.max(time);
                }
                Arc::new(slot)
            };
            let device = match page
                .devices
                .iter_mut()
                .find(|p| p.state.pad.instance == slot.pad.instance)
            {
                Some(device) => {
                    if first_gesture.is_some()
                        || device.state.pad != slot.pad
                        || slot.timestamp > device.state.timestamp
                    {
                        device.state = snapshot();
                        if let Some(object) = device.event_pad {
                            unsafe { objects::refresh(g, object, device.state.clone()) };
                        }
                    }
                    device
                }
                None => {
                    page.devices.push(Device {
                        state: snapshot(),
                        event_pad: None,
                    });
                    page.devices.last_mut().unwrap()
                }
            };
            if page.exposed && device.event_pad.is_none() {
                let obj = unsafe { objects::pad(g, device.state.clone(), true) };
                unsafe { redjs_protect(g, obj) };
                device.event_pad = Some(obj);
                added.push(obj);
            }
        }
        page.devices.sort_unstable_by_key(|d| d.state.index);
        added
    });
    for object in added {
        unsafe { objects::dispatch(g, "gamepadconnected", object) };
    }
}

fn usable(instance: u32) -> bool {
    PAGE.with_borrow(|page| {
        page.focused
            && page.exposed
            && page.disconnecting != Some(instance)
            && page
                .devices
                .iter()
                .any(|d| d.state.pad.instance == instance)
    })
}

pub(super) fn pending() -> bool {
    haptics::pending()
}
pub(super) fn tick(rt: &JsRuntime) {
    haptics::tick(rt.raw())
}
pub(super) fn preempt(rt: &JsRuntime, instance: u32) {
    haptics::preempt(rt.raw(), instance)
}

pub(super) fn shutdown(rt: &JsRuntime) {
    let g = rt.raw();
    SESSION.set(SESSION.get().wrapping_add(1));
    objects::clear_handlers(g);
    let old = PAGE.take();
    for device in old.devices {
        if let Some(object) = device.event_pad {
            unsafe {
                objects::set_connected(g, object, false);
                redjs_unprotect(g, object);
            }
        }
    }
    haptics::cancel_unavailable(g);
}
