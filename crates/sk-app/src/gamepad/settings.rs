use super::native::{field, invalid};
use super::{activate, SERVICE};
use crate::js::{self, Context, Value};
use crate::tray::Roots;
use sk_gamepad::DeadZones;
use std::collections::BTreeMap;
use std::time::Instant;

enum Target {
    Global,
    Model(u16, u16),
    Device(String),
    Connection(u32),
}

impl Target {
    fn parse(ctx: &Context, target: Value) -> js::Result<Self> {
        if ctx.kind(target) != 8 {
            return Err(invalid(ctx, "Expected a dead zone target object"));
        }
        let get = |name| ctx.get(target, &ctx.atom(name));
        let scope = get("scope")?;
        if ctx.kind(scope) != 5 {
            return Err(invalid(ctx, "Expected a dead zone scope"));
        }
        match ctx.read_string(scope)?.as_str() {
            "global" => Ok(Self::Global),
            "model" => {
                let vendor = integer(ctx, get("vendorId")?, "vendorId", 1, u16::MAX.into())?;
                let product = integer(ctx, get("productId")?, "productId", 0, u16::MAX.into())?;
                let (vendor, product) = DeadZones::model_key(vendor as u16, product as u16);
                Ok(Self::Model(vendor, product))
            }
            "device" => {
                let id = get("id")?;
                let connection = if id.is_undefined() {
                    None
                } else {
                    Some(integer(ctx, id, "id", 1, u32::MAX)?)
                };
                let system_id = get("systemId")?;
                if let Some(id) = connection {
                    if !system_id.is_undefined() {
                        return Err(invalid(ctx, "Choose either id or systemId"));
                    }
                    return Self::connection(id)
                        .ok_or_else(|| invalid(ctx, "Controller id is not connected"));
                }
                if ctx.kind(system_id) != 5 {
                    return Err(invalid(ctx, "Expected a controller systemId or id"));
                }
                let key = ctx.read_string(system_id)?;
                if key.is_empty() {
                    return Err(invalid(ctx, "systemId must be a non-empty string"));
                }
                Ok(Self::Device(key))
            }
            _ => Err(invalid(ctx, "Unknown dead zone scope")),
        }
    }

    fn connection(id: u32) -> Option<Self> {
        let target = |pad: &sk_gamepad::Pad| {
            (pad.instance == id).then(|| match &pad.system_id {
                Some(key) => Self::Device(key.clone()),
                None => Self::Connection(id),
            })
        };
        let now = Instant::now();
        SERVICE.with_borrow_mut(|s| {
            if s.focused {
                s.sample(now);
                s.slots.iter().find_map(|slot| target(&slot.pad))
            } else {
                // The last published snapshot stays frozen on blur, even across a disconnect.
                s.maintain_idle(now);
                s.backend.as_ref()?.pads().find_map(|(_, pad)| target(pad))
            }
        })
    }

    fn get(&self, settings: &DeadZones) -> Option<f64> {
        match self {
            Self::Global => settings.global,
            Self::Model(v, p) => settings.models.get(&(*v, *p)).copied(),
            Self::Device(id) => settings.devices.get(id).copied(),
            Self::Connection(id) => settings.connections.get(id).copied(),
        }
    }

    fn set(self, settings: &mut DeadZones, radius: Option<f64>) {
        fn update<K: Ord>(map: &mut BTreeMap<K, f64>, key: K, value: Option<f64>) {
            if let Some(value) = value {
                map.insert(key, value);
            } else {
                map.remove(&key);
            }
        }
        match self {
            Self::Global => settings.global = radius,
            Self::Model(v, p) => update(&mut settings.models, (v, p), radius),
            Self::Device(id) => update(&mut settings.devices, id, radius),
            Self::Connection(id) => update(&mut settings.connections, id, radius),
        }
    }
}

fn integer(ctx: &Context, value: Value, name: &str, min: u32, max: u32) -> js::Result<u32> {
    value
        .as_number()
        .filter(|n| n.fract() == 0.0 && (f64::from(min)..=f64::from(max)).contains(n))
        .map(|n| n as u32)
        .ok_or_else(|| {
            invalid(
                ctx,
                &format!("{name} must be an integer from {min} to {max}"),
            )
        })
}

pub(super) fn install(ctx: &Context, obj: Value, roots: &mut Roots<'_>) -> js::Result<()> {
    let get = roots.keep(js::function(ctx, "getDeadZone", 1, |ctx, args| {
        let target = Target::parse(ctx, args.arg(0))?;
        let value = SERVICE.with_borrow(|s| target.get(&s.dead_zones));
        Ok(value.map_or_else(Value::null, Value::number))
    })?);
    field(ctx, obj, "getDeadZone", get)?;
    let set = roots.keep(js::function(ctx, "setDeadZone", 2, |ctx, args| {
        let raw = args.arg(1);
        let radius = if raw.is_null() {
            None
        } else {
            Some(
                raw.as_number()
                    .filter(|n| (0.0..1.0).contains(n))
                    .ok_or_else(|| {
                        invalid(
                            ctx,
                            "Dead zone must be null or a number from 0 up to (excluding) 1",
                        )
                    })?,
            )
        };
        let target = Target::parse(ctx, args.arg(0))?;
        SERVICE.with_borrow_mut(|s| {
            if target.get(&s.dead_zones) == radius {
                return;
            }
            target.set(&mut s.dead_zones, radius);
            if let Some(backend) = s.backend.as_mut() {
                backend.set_dead_zones(s.dead_zones.clone());
                s.last_sample = None;
                if s.focused {
                    s.sample(Instant::now());
                }
                activate();
            }
        });
        Ok(Value::undefined())
    })?);
    field(ctx, obj, "setDeadZone", set)
}
