use super::{Sample, Slot};
use crate::js::{self, Context, Value};
use crate::tray::Roots;
use sk_gamepad::PadEvent;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

const BUTTONS: &[&str] = &[
    "south",
    "east",
    "west",
    "north",
    "left-bumper",
    "right-bumper",
    "left-trigger",
    "right-trigger",
    "select",
    "start",
    "left-stick",
    "right-stick",
    "dpad-up",
    "dpad-down",
    "dpad-left",
    "dpad-right",
    "home",
];
const AXES: &[&str] = &["left-x", "left-y", "right-x", "right-y"];

thread_local! {
    static LISTENERS: RefCell<BTreeMap<u64, Value>> = const { RefCell::new(BTreeMap::new()) };
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
}

pub(super) fn observed() -> bool {
    LISTENERS.with_borrow(|listeners| !listeners.is_empty())
}

pub(super) fn listeners() -> Vec<u64> {
    LISTENERS.with_borrow(|all| all.keys().copied().collect())
}

pub(super) fn clear(ctx: &Context) {
    for cb in LISTENERS.take().into_values() {
        ctx.unprotect(cb);
    }
}

pub(super) fn invalid(ctx: &Context, message: &str) -> js::Error {
    js::Error::Exception(ctx.error(1, message))
}

pub(super) fn field(ctx: &Context, obj: Value, name: &str, value: Value) -> js::Result<()> {
    ctx.define(obj, &ctx.atom(name), value, 1 | 4)
}

fn snapshot(ctx: &Context, roots: &mut Roots<'_>, slot: &Slot) -> js::Result<Value> {
    let pad = &slot.pad;
    let obj = roots.keep(ctx.object());
    field(ctx, obj, "id", Value::number(pad.instance.into()))?;
    field(
        ctx,
        obj,
        "systemId",
        pad.system_id
            .as_deref()
            .map_or_else(Value::null, |id| ctx.string(id)),
    )?;
    field(ctx, obj, "deadZone", Value::number(pad.dead_zone))?;
    field(ctx, obj, "index", Value::number(slot.index.into()))?;
    field(ctx, obj, "name", ctx.string(&pad.name))?;
    field(ctx, obj, "family", ctx.string(pad.family))?;
    field(ctx, obj, "vendorId", Value::number(pad.vendor.into()))?;
    field(ctx, obj, "productId", Value::number(pad.product.into()))?;
    field(ctx, obj, "timestamp", Value::number(slot.timestamp))?;
    field(
        ctx,
        obj,
        "mapping",
        ctx.string(if pad.standard { "standard" } else { "" }),
    )?;
    field(ctx, obj, "canRumble", Value::boolean(pad.can_rumble))?;
    for (name, raw, names, values) in [
        ("buttons", "buttonValues", BUTTONS, &pad.buttons),
        ("axes", "axisValues", AXES, &pad.axes),
    ] {
        let named = roots.keep(ctx.object());
        let array = roots.keep(ctx.array(values.len() as u32)?);
        for (i, &v) in values.iter().enumerate() {
            ctx.set_index(array, i as u32, Value::number(v))?;
            if let Some(key) = names.get(i).filter(|_| pad.standard) {
                field(ctx, named, key, Value::number(v))?;
            }
        }
        field(ctx, obj, raw, array)?;
        field(ctx, obj, name, named)?;
    }
    Ok(obj)
}

#[derive(Clone, Copy)]
enum Change {
    Connected,
    Disconnected,
    Button(usize, f64, f64),
    Axis(usize, f64),
}

fn send(
    ctx: &Context,
    listeners: &[u64],
    pad: Value,
    standard: bool,
    change: Change,
) -> js::Result<()> {
    let mut roots = Roots::new(ctx);
    let event = roots.keep(ctx.object());
    let kind = match change {
        Change::Connected => "connected",
        Change::Disconnected => "disconnected",
        Change::Button(..) => "button",
        Change::Axis(..) => "axis",
    };
    field(ctx, event, "type", ctx.string(kind))?;
    field(ctx, event, "gamepad", pad)?;
    let control = match change {
        Change::Button(i, value, old) => {
            field(ctx, event, "pressed", Value::boolean(value > 0.5))?;
            let edge = match (old > 0.5, value > 0.5) {
                (false, true) => ctx.string("down"),
                (true, false) => ctx.string("up"),
                _ => Value::null(),
            };
            field(ctx, event, "edge", edge)?;
            Some((BUTTONS, i, value))
        }
        Change::Axis(i, value) => Some((AXES, i, value)),
        _ => None,
    };
    if let Some((names, index, value)) = control {
        field(ctx, event, "index", Value::number(index as f64))?;
        field(ctx, event, "value", Value::number(value))?;
        field(
            ctx,
            event,
            kind,
            names
                .get(index)
                .filter(|_| standard)
                .map_or_else(Value::null, |s| ctx.string(s)),
        )?;
    }
    for id in listeners {
        if let Some(cb) = LISTENERS.with_borrow(|all| all.get(id).copied()) {
            ctx.protect(cb);
            if let Err(error) = ctx.call(cb, Value::undefined(), &[event]) {
                let message = match error {
                    js::Error::Exception(v) => ctx.error_info(v).0,
                    other => other.to_string(),
                };
                sk_tree::log::record("error", format!("gamepad listener: {message}"));
            }
            ctx.unprotect(cb);
        }
    }
    Ok(())
}

fn diff_values(
    changes: &mut Vec<Change>,
    old: &[f64],
    new: &[f64],
    change: impl Fn(usize, f64, f64) -> Change,
) {
    for (index, &value) in new.iter().enumerate() {
        let before = old.get(index).copied().unwrap_or(0.0);
        if before != value {
            changes.push(change(index, before, value));
        }
    }
}

fn emit(
    ctx: &Context,
    listeners: &[u64],
    slot: &Slot,
    changes: impl IntoIterator<Item = Change>,
) -> js::Result<()> {
    let mut changes = changes.into_iter().peekable();
    if changes.peek().is_none() {
        return Ok(());
    }
    let mut roots = Roots::new(ctx);
    let pad = snapshot(ctx, &mut roots, slot)?;
    for change in changes {
        send(ctx, listeners, pad, slot.pad.standard, change)?;
    }
    Ok(())
}

fn set_value(values: &mut Vec<f64>, index: usize, value: f64) {
    if values.len() <= index {
        values.resize(index + 1, 0.0);
    }
    values[index] = value;
}

pub(super) fn deliver(
    ctx: &Context,
    listeners: &[u64],
    old: &[Slot],
    slots: &[Slot],
    inputs: bool,
    samples: &[Sample],
) {
    if samples.is_empty() && std::ptr::eq(old, slots) {
        return;
    }
    let run = || -> js::Result<()> {
        let mut replay: BTreeMap<_, _> = old.iter().map(|s| (s.pad.instance, s.clone())).collect();
        if inputs {
            for sample in samples {
                for event in &sample.events {
                    let instance = event.instance();
                    let (slot, change) = match event {
                        PadEvent::DeadZone { radius, .. } => {
                            if let Some(slot) = replay.get_mut(&instance) {
                                slot.pad.dead_zone = *radius;
                            }
                            continue;
                        }
                        PadEvent::Connected { index, pad } => {
                            let std::collections::btree_map::Entry::Vacant(entry) =
                                replay.entry(instance)
                            else {
                                continue;
                            };
                            let slot = entry.insert(Slot {
                                index: *index,
                                pad: pad.clone(),
                                timestamp: sample.timestamp,
                            });
                            (slot, Change::Connected)
                        }
                        PadEvent::Disconnected(pad) => {
                            let Some(slot) = replay.get_mut(&instance) else {
                                continue;
                            };
                            slot.pad = pad.clone();
                            (slot, Change::Disconnected)
                        }
                        PadEvent::Button(b) => {
                            let Some(slot) = replay.get_mut(&instance) else {
                                continue;
                            };
                            set_value(&mut slot.pad.buttons, b.index, b.value);
                            (slot, Change::Button(b.index, b.value, b.before))
                        }
                        PadEvent::Axis { index, value, .. } => {
                            let Some(slot) = replay.get_mut(&instance) else {
                                continue;
                            };
                            set_value(&mut slot.pad.axes, *index, *value);
                            (slot, Change::Axis(*index, *value))
                        }
                    };
                    slot.timestamp = sample.timestamp;
                    emit(ctx, listeners, slot, [change])?;
                    if matches!(change, Change::Disconnected) {
                        replay.remove(&instance);
                    }
                }
            }
        }
        // A parked or unfocused host drains OS input privately. On return announce the devices
        // that remain connected, even if their original hotplug events belonged to that interval.
        for gone in replay
            .values()
            .filter(|s| slots.iter().all(|p| p.pad.instance != s.pad.instance))
        {
            emit(ctx, listeners, gone, [Change::Disconnected])?;
        }
        for slot in slots {
            let Some(before) = replay.get(&slot.pad.instance) else {
                emit(ctx, listeners, slot, [Change::Connected])?;
                continue;
            };
            if inputs {
                let mut diffs = Vec::new();
                diff_values(
                    &mut diffs,
                    &before.pad.buttons,
                    &slot.pad.buttons,
                    |i, old, v| Change::Button(i, v, old),
                );
                diff_values(&mut diffs, &before.pad.axes, &slot.pad.axes, |i, _, v| {
                    Change::Axis(i, v)
                });
                emit(ctx, listeners, slot, diffs)?;
            }
        }

        Ok(())
    };
    if let Err(e) = run() {
        log::warn!("Could not deliver gamepad event: {e}");
    }
}

fn option(ctx: &Context, opts: Value, key: &str, default: f64, max: f64) -> js::Result<f64> {
    if opts.is_undefined() {
        return Ok(default);
    }
    let v = ctx.get(opts, &ctx.atom(key))?;
    if v.is_undefined() {
        return Ok(default);
    }
    v.as_number()
        .filter(|n| (0.0..=max).contains(n))
        .ok_or_else(|| invalid(ctx, &format!("{key} must be a number from 0 to {max}")))
}

pub(super) fn install(ctx: &Context) -> js::Result<()> {
    let mut roots = Roots::new(ctx);
    let obj = roots.keep(ctx.object());
    field(ctx, obj, "available", Value::boolean(true))?;
    let list = roots.keep(js::function(ctx, "list", 0, |ctx, _| {
        let slots = super::snapshot();
        let mut roots = Roots::new(ctx);
        let array = roots.keep(ctx.array(slots.len() as u32)?);
        for (i, slot) in slots.iter().enumerate() {
            let pad = snapshot(ctx, &mut roots, slot)?;
            ctx.set_index(array, i as u32, pad)?;
        }
        Ok(array)
    })?);
    field(ctx, obj, "list", list)?;
    super::settings::install(ctx, obj, &mut roots)?;
    let off = roots.keep(js::function(ctx, "unsubscribe", 1, |ctx, args| {
        if let Some(id) = args.arg(0).as_number() {
            if let Some(cb) = LISTENERS.with_borrow_mut(|all| all.remove(&(id as u64))) {
                ctx.unprotect(cb);
            }
        }
        Ok(Value::undefined())
    })?);
    let bind = roots.keep(ctx.get(off, &ctx.atom("bind"))?);
    let subscribe = roots.keep(js::function(ctx, "subscribe", 1, move |ctx, args| {
        let cb = args.arg(0);
        if ctx.kind(cb) != 9 {
            return Err(invalid(ctx, "subscribe requires a function"));
        }
        if LISTENERS.with_borrow(|all| all.len() >= 1024) {
            return Err(invalid(ctx, "Too many gamepad listeners"));
        }
        let id = NEXT_ID.get();
        if id >= (1 << 53) {
            return Err(invalid(ctx, "Gamepad subscription IDs exhausted"));
        }
        NEXT_ID.set(id + 1);
        let unsubscribe = ctx.call(bind, off, &[Value::undefined(), Value::number(id as f64)])?;
        if !observed() {
            super::start_native_stream();
        }
        ctx.protect(cb);
        LISTENERS.with_borrow_mut(|all| all.insert(id, cb));
        super::activate();
        Ok(unsubscribe)
    })?);
    field(ctx, obj, "subscribe", subscribe)?;
    let rumble = roots.keep(js::function(ctx, "rumble", 2, |ctx, args| {
        let id =
            args.arg(0)
                .as_number()
                .filter(|n| n.fract() == 0.0 && (1.0..=u32::MAX.into()).contains(n))
                .ok_or_else(|| invalid(ctx, "rumble requires a controller id"))? as u32;
        let opts = args.arg(1);
        if !opts.is_undefined() && ctx.kind(opts) != 8 {
            return Err(invalid(ctx, "rumble options must be an object"));
        }
        let strong = option(ctx, opts, "strong", 1.0, 1.0)?;
        let weak = option(ctx, opts, "weak", 1.0, 1.0)?;
        let duration = option(ctx, opts, "durationMs", 120.0, 5000.0)? as u32;
        sk_runtime::JsRuntime::with(|rt| super::web::preempt(rt, id));
        Ok(Value::boolean(super::rumble(id, strong, weak, duration)))
    })?);
    field(ctx, obj, "rumble", rumble)?;
    field(ctx, ctx.global_get("__sk_std")?, "gamepads", obj)?;
    // Native closures are opaque to the JS collector.
    ctx.protect(off);
    ctx.protect(bind);
    crate::js::on_shutdown(move |ctx| {
        ctx.unprotect(off);
        ctx.unprotect(bind);
    });
    Ok(())
}
