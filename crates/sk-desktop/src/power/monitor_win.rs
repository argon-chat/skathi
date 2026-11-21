use std::ffi::c_void;
use std::sync::Mutex;

use windows::core::GUID;
use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::System::Power::{
    CallNtPowerInformation, GetSystemPowerStatus, PowerSettingRegisterNotification,
    ProcessorInformation, DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, POWERBROADCAST_SETTING,
    PROCESSOR_POWER_INFORMATION, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemServices::{
    GUID_MAX_POWER_SAVINGS, GUID_MIN_POWER_SAVINGS, GUID_POWERSCHEME_PERSONALITY,
};
use windows::Win32::System::Threading::{
    GetActiveProcessorCount, GetSystemTimes, ALL_PROCESSOR_GROUPS,
};
use windows::Win32::UI::WindowsAndMessaging::{DEVICE_NOTIFY_CALLBACK, PBT_POWERSETTINGCHANGE};

use super::{set_scheme, CpuTimes, Scheme};

const ENERGY_SAVER: GUID = GUID::from_u128(0x550e8400_e29b_41d4_a716_446655440000);
const BATTERY_SAVER: GUID = GUID::from_u128(0xe00958c0_c213_4ace_ac77_fecced2eeea5);

#[derive(Default)]
struct Settings {
    energy_saver: Option<bool>,
    battery_saver: bool,
    profile: Scheme,
}

impl Settings {
    fn scheme(&self) -> Scheme {
        if self.energy_saver.unwrap_or(self.battery_saver) {
            Scheme::Saving
        } else {
            self.profile
        }
    }
}

static SETTINGS: Mutex<Settings> = Mutex::new(Settings {
    energy_saver: None,
    battery_saver: false,
    profile: Scheme::Balanced,
});

unsafe extern "system" fn changed(_: *const c_void, kind: u32, data: *const c_void) -> u32 {
    if kind != PBT_POWERSETTINGCHANGE || data.is_null() {
        return 0;
    }
    // SAFETY: the callback receives a POWERBROADCAST_SETTING followed by DataLength bytes.
    let setting = data.cast::<POWERBROADCAST_SETTING>();
    let guid = unsafe { (*setting).PowerSetting };
    let size = unsafe { (*setting).DataLength } as usize;
    let payload = unsafe { std::ptr::addr_of!((*setting).Data) }.cast::<u8>();
    let mut state = SETTINGS.lock().unwrap();
    match guid {
        ENERGY_SAVER if size == size_of::<u32>() => {
            let status = unsafe { payload.cast::<u32>().read_unaligned() };
            state.energy_saver = match status {
                0 => Some(false),
                1 | 2 => Some(true),
                _ => None,
            };
        }
        BATTERY_SAVER if size == size_of::<u32>() => {
            state.battery_saver = unsafe { payload.cast::<u32>().read_unaligned() } == 1;
        }
        GUID_POWERSCHEME_PERSONALITY if size == size_of::<GUID>() => {
            state.profile = match unsafe { payload.cast::<GUID>().read_unaligned() } {
                GUID_MAX_POWER_SAVINGS => Scheme::Saving,
                GUID_MIN_POWER_SAVINGS => Scheme::Performance,
                _ => Scheme::Balanced,
            };
        }
        _ => return 0,
    }
    set_scheme(state.scheme());
    0
}

pub(super) fn subscribe() {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: the status is writable; the callback and its null context are process-lifetime.
    unsafe {
        if GetSystemPowerStatus(&mut status).is_ok() {
            let mut state = SETTINGS.lock().unwrap();
            state.battery_saver = status.SystemStatusFlag == 1;
            set_scheme(state.scheme());
        }
        let params = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(changed),
            Context: std::ptr::null_mut(),
        };
        let recipient = HANDLE((&raw const params).cast_mut().cast());
        for guid in [BATTERY_SAVER, ENERGY_SAVER, GUID_POWERSCHEME_PERSONALITY] {
            let mut registration = std::ptr::null_mut();
            let _ = PowerSettingRegisterNotification(
                &guid,
                DEVICE_NOTIFY_CALLBACK,
                recipient,
                &mut registration,
            );
        }
    }
}

pub(super) fn cpu_times() -> Option<CpuTimes> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the three FILETIME outputs are distinct writable storage.
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
    let ticks = |s: FILETIME| (u64::from(s.dwHighDateTime) << 32) | u64::from(s.dwLowDateTime);
    let total = ticks(kernel) + ticks(user);
    Some(CpuTimes {
        total,
        busy: total.checked_sub(ticks(idle))?,
    })
}

pub(super) fn frequency_limit(usage: f64) -> Option<f64> {
    // SAFETY: ALL_PROCESSOR_GROUPS requests a count; no pointers are passed.
    let n = unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) } as usize;
    let mut data = vec![PROCESSOR_POWER_INFORMATION::default(); n];
    let bytes = u32::try_from(size_of_val(data.as_slice())).ok()?;
    // SAFETY: data contains exactly n records and is exclusive for the call.
    if unsafe {
        CallNtPowerInformation(
            ProcessorInformation,
            None,
            0,
            Some(data.as_mut_ptr().cast()),
            bytes,
        )
    }
    .0 != 0
    {
        return None;
    }
    let (capacity, maximum) = data.iter().filter(|core| core.MaxMhz > 0).fold(
        (0u64, 0u64),
        |(capacity, maximum), core| {
            let limit = if core.MhzLimit > 0 {
                core.MhzLimit
            } else {
                core.MaxMhz
            };
            // Below high utilization, the governor can lower the current clock without throttling.
            let available = if usage >= 0.8 && core.CurrentMhz > 0 {
                limit.min(core.CurrentMhz)
            } else {
                limit
            };
            (
                capacity + u64::from(available),
                maximum + u64::from(core.MaxMhz),
            )
        },
    );
    (maximum > 0).then(|| capacity as f64 / maximum as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_energy_saver_overrides_the_legacy_status_and_profile() {
        let mut settings = Settings::default();
        assert_eq!(settings.scheme(), Scheme::Balanced);
        settings.battery_saver = true;
        assert_eq!(settings.scheme(), Scheme::Saving);
        settings.energy_saver = Some(false);
        settings.profile = Scheme::Performance;
        assert_eq!(settings.scheme(), Scheme::Performance);
        settings.energy_saver = Some(true);
        assert_eq!(settings.scheme(), Scheme::Saving);
    }
}
