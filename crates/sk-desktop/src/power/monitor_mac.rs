use std::sync::OnceLock;

use block2::RcBlock;
use objc2_foundation::{
    NSNotificationCenter, NSProcessInfo, NSProcessInfoPowerStateDidChangeNotification,
    NSProcessInfoThermalState, NSProcessInfoThermalStateDidChangeNotification,
};

use super::{set_scheme, set_thermal, CpuTimes, Pressure, Scheme};

fn refresh_thermal() {
    let state = match NSProcessInfo::processInfo().thermalState() {
        NSProcessInfoThermalState::Fair => Pressure::Fair,
        NSProcessInfoThermalState::Serious => Pressure::Serious,
        NSProcessInfoThermalState::Critical => Pressure::Critical,
        _ => Pressure::Nominal,
    };
    set_thermal(state);
}

fn refresh_scheme() {
    set_scheme(if NSProcessInfo::processInfo().isLowPowerModeEnabled() {
        Scheme::Saving
    } else {
        Scheme::Balanced
    });
}

pub(super) fn subscribe() {
    let center = NSNotificationCenter::defaultCenter();
    // Foundation requires reading thermalState before registering for its notification.
    refresh_thermal();
    // SAFETY: the blocks capture no data and only update thread-safe state; the center copies them.
    unsafe {
        let _ = center.addObserverForName_object_queue_usingBlock(
            Some(NSProcessInfoThermalStateDidChangeNotification),
            None,
            None,
            &RcBlock::new(|_| refresh_thermal()),
        );
        let _ = center.addObserverForName_object_queue_usingBlock(
            Some(NSProcessInfoPowerStateDidChangeNotification),
            None,
            None,
            &RcBlock::new(|_| refresh_scheme()),
        );
    }
    refresh_thermal();
    refresh_scheme();
}

pub(super) fn cpu_times() -> Option<CpuTimes> {
    static PORT: OnceLock<libc::mach_port_t> = OnceLock::new();
    // SAFETY: the acquired Mach host send right is retained for the process lifetime.
    let host = *PORT.get_or_init(|| unsafe { libc::mach_host_self() });
    let mut ticks = [0u32; libc::HOST_CPU_LOAD_INFO_COUNT as usize];
    let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
    // SAFETY: HOST_CPU_LOAD_INFO writes four integer counters, matching ticks and count.
    let result = unsafe {
        libc::host_statistics(
            host,
            libc::HOST_CPU_LOAD_INFO,
            ticks.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if result != 0 || count != libc::HOST_CPU_LOAD_INFO_COUNT {
        return None;
    }
    let [user, system, idle, nice] = ticks.map(u64::from);
    let busy = user + system + nice;
    Some(CpuTimes {
        busy,
        total: busy + idle,
    })
}
