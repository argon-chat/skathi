use super::*;
use sk_runtime::redjs::{glue, web::now_ms};
use std::collections::BTreeMap;

struct Effect {
    start: f64,
    duration: u32,
    strong: f64,
    weak: f64,
    started: bool,
    promise: host::Settler,
}
thread_local! { static EFFECTS: RefCell<BTreeMap<u32, Effect>> = const { RefCell::new(BTreeMap::new()) }; }

pub(super) fn pending() -> bool {
    EFFECTS.with_borrow(|effects| !effects.is_empty())
}

pub(super) fn preempt(g: *mut c_void, instance: u32) {
    if let Some(effect) = EFFECTS.with_borrow_mut(|e| e.remove(&instance)) {
        if effect.started {
            super::super::rumble(instance, 0., 0., 0);
        }
        unsafe { effect.promise.resolve(g, js_string(g, "preempted")) };
    }
}

pub(super) fn cancel_unavailable(g: *mut c_void) {
    let removed: Vec<_> =
        EFFECTS.with_borrow(|effects| effects.keys().copied().filter(|id| !usable(*id)).collect());
    for instance in removed {
        preempt(g, instance);
    }
}

pub(super) fn tick(g: *mut c_void) {
    let now = now_ms();
    let done = EFFECTS.with_borrow_mut(|effects| {
        let mut done = Vec::new();
        for (&instance, effect) in effects.iter_mut() {
            if now < effect.start {
                continue;
            }
            if !effect.started && effect.duration > 0 {
                if !super::super::rumble(instance, effect.strong, effect.weak, effect.duration) {
                    done.push((instance, false));
                    continue;
                }
                // Device playback starts when the command is actually sent.
                effect.start = now;
                effect.started = true;
            }
            if now >= effect.start + f64::from(effect.duration) {
                done.push((instance, true));
            }
        }
        done.into_iter()
            .filter_map(|(instance, ok)| effects.remove(&instance).map(|e| (e.promise, ok)))
            .collect::<Vec<_>>()
    });
    for (promise, ok) in done {
        unsafe {
            if ok {
                promise.resolve(g, js_string(g, "complete"));
            } else {
                promise.reject(
                    g,
                    host::error(
                        g,
                        "NotSupportedError",
                        "The controller could not play the effect",
                    ),
                );
            }
        }
    }
}

unsafe fn parameter(g: *mut c_void, init: u64, name: &str, max: f64) -> Result<f64, u64> {
    if matches!(init, NULL | UNDEF) {
        return Ok(0.);
    }
    unsafe {
        let key = atom(g, name);
        let mut value = UNDEF;
        let status = glue::rjs_get(g, init, key, &mut value);
        glue::rjs_atom_free(g, key);
        if status != 0 {
            return Err(value);
        }
        if value == UNDEF {
            return Ok(0.);
        }
        let mut number = 0.;
        let mut error = UNDEF;
        if glue::rjs_to_number_x(g, value, &mut number, &mut error) != 0 {
            return Err(error);
        }
        if !(0.0..=max).contains(&number) {
            return Err(host::error(
                g,
                "TypeError",
                &format!("{name} must be between 0 and {max}"),
            ));
        }
        Ok(number)
    }
}

host_fn!(pub(super) fn play_effect(g, args) {
    unsafe {
        let Some(actuator) = objects::actuator(g, args.this().0) else { return rejected_promise(g,1,"Illegal invocation") };
        let mut effect = UNDEF;
        if glue::rjs_to_string(g, args.arg(0).0, &mut effect) != 0 { return host::rejected_with(g, effect) }
        let name = read_string(g, effect).unwrap_or_default();
        if !matches!(name.as_str(), "dual-rumble" | "trigger-rumble") { return rejected_promise(g,1,"Invalid GamepadHapticEffectType"); }
        let init = args.arg(1).0;
        if !matches!(init, NULL | UNDEF) && glue::rjs_kind(g, init) < 8 { return rejected_promise(g,1,"Expected GamepadEffectParameters"); }
        let params = (|| {
            let duration = parameter(g, init, "duration", 5000.)?;
            let delay = parameter(g, init, "startDelay", 5000.)?;
            if duration + delay > 5000. { return Err(host::error(g, "TypeError", "Gamepad effects can run for up to 5000 ms including startDelay")); }
            let strong = parameter(g, init, "strongMagnitude", 1.)?;
            let weak = parameter(g, init, "weakMagnitude", 1.)?;
            if name == "trigger-rumble" {
                parameter(g, init, "leftTrigger", 1.)?;
                parameter(g, init, "rightTrigger", 1.)?;
            }
            Ok((duration as u32, delay, strong, weak))
        })();
        let (duration, delay, strong, weak) = match params { Ok(p) => p, Err(error) => return host::rejected_with(g, error) };
        if !actuator.active() { return host::rejected(g, "InvalidStateError", "The controller is disconnected or the window is inactive"); }
        preempt(g, actuator.instance);
        if !actuator.supported || name != "dual-rumble" { return host::rejected(g, "NotSupportedError", "The controller does not support this effect"); }
        let (result, promise) = host::deferred(g);
        EFFECTS.with_borrow_mut(|effects| effects.insert(actuator.instance, Effect { start: now_ms() + delay, duration, strong, weak, started:false, promise }));
        tick(g);
        sk_runtime::frames::wake();
        result
    }
});

host_fn!(pub(super) fn reset(g, args) {
    unsafe {
        let Some(actuator) = objects::actuator(g,args.this().0) else { return rejected_promise(g,1,"Illegal invocation") };
        if !actuator.active() { return host::rejected(g,"InvalidStateError","The controller is disconnected or the window is inactive") }
        preempt(g, actuator.instance);
        if actuator.supported { super::super::rumble(actuator.instance, 0., 0., 0); }
        resolved_promise(g, js_string(g, "complete"))
    }
});
