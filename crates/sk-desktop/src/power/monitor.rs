use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
use std::sync::{Condvar, Mutex, Once};
use std::time::Duration;

#[cfg(target_os = "linux")]
#[path = "monitor_linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "monitor_mac.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "monitor_win.rs"]
mod platform;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Pressure {
    #[default]
    Nominal,
    Fair,
    Serious,
    Critical,
}

impl Pressure {
    fn from_level(level: u8) -> Self {
        match level {
            1 => Self::Fair,
            2 => Self::Serious,
            3 => Self::Critical,
            _ => Self::Nominal,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Scheme {
    #[default]
    Balanced,
    Saving,
    Performance,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PressureSample {
    pub serial: u64,
    pub state: Pressure,
    pub time: f64,
}

static SCHEME: AtomicU8 = AtomicU8::new(Scheme::Balanced as u8);
static THERMAL: AtomicU8 = AtomicU8::new(Pressure::Nominal as u8);
static INIT: Once = Once::new();
static CHANGED: Condvar = Condvar::new();
static SAMPLER: Mutex<Sampler> = Mutex::new(Sampler {
    demand: false,
    repeat: false,
    running: false,
    first: true,
    hardware: Pressure::Nominal,
    latest: PressureSample {
        serial: 0,
        state: Pressure::Nominal,
        time: 0.,
    },
});

struct Sampler {
    demand: bool,
    repeat: bool,
    running: bool,
    first: bool,
    hardware: Pressure,
    latest: PressureSample,
}

impl Sampler {
    fn publish(&mut self) -> bool {
        let thermal = Pressure::from_level(THERMAL.load(Relaxed));
        let state = self.hardware.max(thermal);
        let changed = std::mem::take(&mut self.first) || self.latest.state != state;
        self.latest = PressureSample {
            serial: self.latest.serial.wrapping_add(1),
            state,
            time: sk_runtime::redjs::web::now_ms(),
        };
        changed || self.repeat
    }
}

pub fn init() {
    INIT.call_once(|| {
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        platform::subscribe();
    });
}

pub fn scheme() -> Scheme {
    match SCHEME.load(Relaxed) {
        1 => Scheme::Saving,
        2 => Scheme::Performance,
        _ => Scheme::Balanced,
    }
}

fn set_scheme(scheme: Scheme) {
    if SCHEME.swap(scheme as u8, Relaxed) != scheme as u8 {
        sk_runtime::frames::wake();
    }
}

#[cfg(target_os = "macos")]
fn set_thermal(thermal: Pressure) {
    if THERMAL.swap(thermal as u8, Relaxed) == thermal as u8 {
        return;
    }
    let mut sampler = SAMPLER.lock().unwrap();
    if sampler.demand && sampler.publish() {
        drop(sampler);
        sk_runtime::frames::wake();
    }
}

pub fn pressure_sample() -> PressureSample {
    SAMPLER.lock().unwrap().latest
}

/// A process-wide sample stream, enabled only by a visible consumer.
pub fn set_pressure_demand(demand: bool, repeat: bool) {
    let mut sampler = SAMPLER.lock().unwrap();
    sampler.repeat = repeat;
    sampler.first |= demand && !sampler.demand;
    sampler.demand = demand;
    if !demand {
        CHANGED.notify_one();
    } else if !sampler.running {
        sampler.running = true;
        if std::thread::Builder::new()
            .name("sk-pressure".into())
            .spawn(sample)
            .is_err()
        {
            sampler.running = false;
        }
    }
}

#[derive(Clone, Copy)]
struct CpuTimes {
    busy: u64,
    total: u64,
}

impl CpuTimes {
    fn usage_since(self, before: Self) -> Option<f64> {
        let total = self.total.checked_sub(before.total)?;
        let busy = self.busy.checked_sub(before.busy)?;
        (total > 0).then(|| (busy as f64 / total as f64).min(1.0))
    }
}

fn pressure_for(usage: f64, limit: Option<f64>, prior: Pressure) -> Pressure {
    let usage = (usage / limit.unwrap_or(1.0).clamp(0.1, 1.0)).min(1.0);
    let margin = |level| if prior >= level { 0.03 } else { 0.0 };
    if usage >= 0.95 - margin(Pressure::Critical) {
        Pressure::Critical
    } else if usage >= 0.80 - margin(Pressure::Serious) {
        Pressure::Serious
    } else if usage >= 0.60 - margin(Pressure::Fair) {
        Pressure::Fair
    } else {
        Pressure::Nominal
    }
}

fn sample() {
    let mut before = cpu_times();
    let mut previous = Pressure::Nominal;
    loop {
        let sampler = SAMPLER.lock().unwrap();
        let (mut sampler, _) = CHANGED
            .wait_timeout_while(sampler, Duration::from_secs(1), |s| s.demand)
            .unwrap();
        if !sampler.demand {
            sampler.running = false;
            sampler.hardware = Pressure::Nominal;
            return;
        }
        drop(sampler);

        let current = cpu_times();
        let usage = current.zip(before).and_then(|(n, p)| n.usage_since(p));
        before = current;
        previous = usage.map_or(previous, |usage| {
            pressure_for(usage, frequency_limit(usage), previous)
        });
        let thermal = thermal_pressure();

        let mut sampler = SAMPLER.lock().unwrap();
        if sampler.demand {
            sampler.hardware = previous.max(thermal);
            if sampler.publish() {
                drop(sampler);
                sk_runtime::frames::wake();
            }
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
use platform::cpu_times;
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn cpu_times() -> Option<CpuTimes> {
    None
}

#[cfg(target_os = "windows")]
use platform::frequency_limit;
#[cfg(not(target_os = "windows"))]
fn frequency_limit(_usage: f64) -> Option<f64> {
    None
}

#[cfg(target_os = "linux")]
use platform::thermal_pressure;
#[cfg(not(target_os = "linux"))]
fn thermal_pressure() -> Pressure {
    Pressure::Nominal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_uses_deltas_capacity_and_hysteresis() {
        assert_eq!(
            CpuTimes {
                busy: 16,
                total: 30
            }
            .usage_since(CpuTimes {
                busy: 10,
                total: 20
            }),
            Some(0.6)
        );
        assert_eq!(
            CpuTimes { busy: 2, total: 2 }.usage_since(CpuTimes { busy: 5, total: 5 }),
            None
        );
        assert_eq!(
            pressure_for(0.30, None, Pressure::Nominal),
            Pressure::Nominal
        );
        assert_eq!(pressure_for(0.65, None, Pressure::Nominal), Pressure::Fair);
        assert_eq!(
            pressure_for(0.84, None, Pressure::Nominal),
            Pressure::Serious
        );
        assert_eq!(
            pressure_for(0.93, None, Pressure::Critical),
            Pressure::Critical
        );
        assert_eq!(
            pressure_for(0.93, None, Pressure::Nominal),
            Pressure::Serious
        );
        assert_eq!(
            pressure_for(0.70, Some(0.70), Pressure::Nominal),
            Pressure::Critical
        );
    }
}
