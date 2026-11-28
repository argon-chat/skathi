//! Native gamepad input and dual-motor vibration.

use gilrs::ff::{BaseEffect, BaseEffectType, Effect, EffectBuilder, Repeat, Replay, Ticks};
use gilrs::{EventType, Gamepad, GamepadId, Gilrs, GilrsBuilder};
use std::sync::atomic::{AtomicU32, Ordering};

mod deadzone;
pub use deadzone::DeadZones;
mod state;
mod system_id;
use state::State;
#[cfg(windows)]
mod steam;

#[derive(Clone, Debug, PartialEq)]
pub struct Pad {
    /// New for each connection, including a reconnection of the same physical device.
    pub instance: u32,
    /// A system device identity independent of this connection, when the OS provides one.
    pub system_id: Option<String>,
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub family: &'static str,
    pub standard: bool,
    /// Effective radial dead zone. Only standard stick axes are filtered.
    pub dead_zone: f64,
    pub axes: Vec<f64>,
    pub buttons: Vec<f64>,
    pub can_rumble: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonChange {
    pub instance: u32,
    pub index: usize,
    pub before: f64,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PadEvent {
    Connected {
        index: u32,
        pad: Pad,
    },
    Disconnected(Pad),
    /// Updates snapshot metadata before axis changes produced by live recalibration.
    DeadZone {
        instance: u32,
        radius: f64,
    },
    Button(ButtonChange),
    Axis {
        instance: u32,
        index: usize,
        /// Effective value immediately before this change.
        before: f64,
        value: f64,
        /// A dead-zone change recomputed this value from a stationary input sample.
        recalibrated: bool,
    },
}

impl PadEvent {
    pub fn instance(&self) -> u32 {
        match self {
            Self::Connected { pad, .. } | Self::Disconnected(pad) => pad.instance,
            Self::Button(change) => change.instance,
            Self::Axis { instance, .. } | Self::DeadZone { instance, .. } => *instance,
        }
    }

    pub fn button(&self) -> Option<ButtonChange> {
        match self {
            Self::Button(change) => Some(*change),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Gilrs(GamepadId),
    #[cfg(windows)]
    Steam(u32),
}

struct Device {
    source: Source,
    index: u32,
    state: State,
    effect: Option<Effect>,
}

// Stale handles must stay invalid even if the host tears down and restarts the backend.
static LAST_INSTANCE: AtomicU32 = AtomicU32::new(0);

fn next_instance() -> Option<u32> {
    LAST_INSTANCE
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            last.checked_add(1)
        })
        .ok()
        .map(|last| last + 1)
}

fn connect(
    devices: &mut Vec<Device>,
    source: Source,
    state: impl FnOnce(u32) -> State,
) -> Option<&Device> {
    let instance = next_instance()?;
    let index = (0..).find(|index| devices.iter().all(|d| d.index != *index))?;
    devices.push(Device {
        source,
        index,
        state: state(instance),
        effect: None,
    });
    devices.last()
}

fn connect_gilrs<'a>(
    devices: &'a mut Vec<Device>,
    gamepad: Gamepad<'_>,
    zones: &DeadZones,
) -> Option<&'a Device> {
    connect(devices, Source::Gilrs(gamepad.id()), |instance| {
        State::connected(instance, &gamepad, zones)
    })
}

fn disconnect(devices: &mut Vec<Device>, source: Source, events: &mut Vec<PadEvent>) {
    if let Some(pos) = devices.iter().position(|d| d.source == source) {
        events.push(PadEvent::Disconnected(devices.remove(pos).state.pad));
    }
}

pub struct Gamepads {
    // Release live vibration effects before dropping the feedback service.
    devices: Vec<Device>,
    gilrs: Gilrs,
    #[cfg(windows)]
    steam: Option<steam::Receiver>,
    events: Vec<PadEvent>,
    publish_initial: bool,
    dead_zones: DeadZones,
    dead_zones_changed: bool,
}

impl Gamepads {
    pub fn new() -> Result<Self, String> {
        let gilrs = GilrsBuilder::new()
            .with_default_filters(false)
            // The IOKit backend cannot vibrate. Its feedback worker would wake up 20 times/sec.
            .with_force_feedback(!cfg!(target_os = "macos"))
            .build()
            .map_err(|error| error.to_string())?;
        let mut devices = Vec::new();
        let dead_zones = DeadZones::default();
        for (_, gamepad) in gilrs.gamepads() {
            let _ = connect_gilrs(&mut devices, gamepad, &dead_zones);
        }
        Ok(Self {
            gilrs,
            #[cfg(windows)]
            steam: steam::Receiver::for_process(),
            devices,
            events: Vec::new(),
            publish_initial: true,
            dead_zones,
            dead_zones_changed: false,
        })
    }

    /// Drains input in order and borrows the resulting snapshots.
    pub fn sample(&mut self) -> impl ExactSizeIterator<Item = &Pad> + Clone {
        self.events.clear();
        if std::mem::take(&mut self.dead_zones_changed) {
            for device in &mut self.devices {
                device.state.configure(&self.dead_zones, &mut self.events);
            }
        }
        if std::mem::take(&mut self.publish_initial) {
            // The first snapshot already contains the configured values.
            self.events.clear();
            self.events
                .extend(self.devices.iter().map(|device| PadEvent::Connected {
                    index: device.index,
                    pad: device.state.pad.clone(),
                }));
        }
        while let Some(event) = self.gilrs.next_event() {
            let source = Source::Gilrs(event.id);
            if event.event == EventType::Disconnected {
                disconnect(&mut self.devices, source, &mut self.events);
                continue;
            }
            if !self.devices.iter().any(|d| d.source == source)
                && let Some(gamepad) = self.gilrs.connected_gamepad(event.id)
                && let Some(device) = connect_gilrs(&mut self.devices, gamepad, &self.dead_zones)
            {
                self.events.push(PadEvent::Connected {
                    index: device.index,
                    pad: device.state.pad.clone(),
                });
            }
            if let Some(device) = self.devices.iter_mut().find(|d| d.source == source) {
                match event.event {
                    // gilrs emits ButtonChanged for both digital edges and analog trigger values.
                    EventType::ButtonChanged(button, value, code) => {
                        device
                            .state
                            .button(button, code.into_u32(), value, &mut self.events);
                    }
                    EventType::AxisChanged(axis, value, code) => {
                        device
                            .state
                            .axis(axis, code.into_u32(), value, &mut self.events);
                    }
                    _ => {}
                }
            }
        }
        #[cfg(windows)]
        if let Some(receiver) = self.steam.as_mut() {
            receiver.poll(&mut self.devices, &mut self.events, &self.dead_zones);
        }
        self.pads().map(|(_, pad)| pad)
    }

    /// Applies the latest settings at the next sample, including when a stick is stationary.
    pub fn set_dead_zones(&mut self, settings: DeadZones) {
        if self.dead_zones != settings {
            self.dead_zones = settings;
            self.dead_zones_changed = true;
        }
    }

    /// Borrows the current snapshots without consuming new input.
    pub fn pads(&self) -> impl ExactSizeIterator<Item = (u32, &Pad)> + Clone {
        self.devices
            .iter()
            .map(|device| (device.index, &device.state.pad))
    }

    /// Ends a host session while retaining the OS device monitor and current physical input state.
    pub fn reset_session(&mut self) {
        let _ = self.sample();
        self.events.clear();
        self.publish_initial = true;
        self.dead_zones.connections.clear();
        self.dead_zones_changed = true;
        self.devices.retain_mut(|device| {
            if let Some(effect) = device.effect.take() {
                let _ = effect.stop();
            }
            let Some(instance) = next_instance() else {
                return false;
            };
            device.state.pad.instance = instance;
            true
        });
    }

    /// Returns the latest sample's connections and input in OS observation order.
    pub fn take_events(&mut self) -> Vec<PadEvent> {
        std::mem::take(&mut self.events)
    }

    /// Drives low- and high-frequency motors. A zero duration or two zero magnitudes stops them.
    pub fn rumble(&mut self, instance: u32, strong: f64, weak: f64, duration_ms: u32) -> bool {
        let Some(device) = self
            .devices
            .iter_mut()
            .find(|d| d.state.pad.instance == instance)
        else {
            return false;
        };
        if !device.state.pad.can_rumble || !strong.is_finite() || !weak.is_finite() {
            return false;
        }
        let id = match device.source {
            Source::Gilrs(id) => id,
            #[cfg(windows)]
            Source::Steam(_) => return false,
        };
        if let Some(previous) = device.effect.take()
            && previous.stop().is_err()
        {
            return false;
        }
        if duration_ms == 0 || (strong <= 0.0 && weak <= 0.0) {
            return true;
        }
        let magnitude = |v: f64| (v.clamp(0.0, 1.0) * f64::from(u16::MAX)).round() as u16;
        let duration = Ticks::from_ms(duration_ms);
        let mut builder = EffectBuilder::new();
        for kind in [
            BaseEffectType::Strong {
                magnitude: magnitude(strong),
            },
            BaseEffectType::Weak {
                magnitude: magnitude(weak),
            },
        ] {
            builder.add_effect(BaseEffect {
                kind,
                scheduling: Replay {
                    play_for: duration,
                    ..Default::default()
                },
                ..Default::default()
            });
        }
        let effect = builder
            .gamepads(&[id])
            .repeat(Repeat::For(duration))
            .finish(&mut self.gilrs);
        match effect {
            Ok(effect) if effect.play().is_ok() => {
                device.effect = Some(effect);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_handles_are_never_reused_by_a_new_backend() {
        let first = next_instance().unwrap();
        assert!(first > 0);
        assert!(next_instance().unwrap() > first);
    }

    #[test]
    #[ignore = "requires a desktop gamepad service"]
    fn enumerate_native_gamepads() {
        let mut service = Gamepads::new().unwrap();
        // WGI publishes devices asynchronously and the Puck supplies its first state after open.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while std::time::Instant::now() < until {
            let _ = service.sample();
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        let pads = service.sample();
        let old_instances: Vec<_> = pads.clone().map(|pad| pad.instance).collect();
        eprintln!("Connected gamepads: {}", pads.len());
        for pad in pads {
            eprintln!("{} ({:04x}:{:04x})", pad.name, pad.vendor, pad.product);
            if pad.standard {
                assert!(pad.axes.len() >= 4);
                assert!(pad.buttons.len() >= 17);
            }
            assert!(pad.axes.iter().all(|axis| (-1.0..=1.0).contains(axis)));
            assert!(pad.buttons.iter().all(|value| (0.0..=1.0).contains(value)));
        }
        service.reset_session();
        assert!(service
            .pads()
            .all(|(_, pad)| !old_instances.contains(&pad.instance)));
        assert!(service.take_events().is_empty());
    }
}
