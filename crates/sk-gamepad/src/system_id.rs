//! Opaque keys for an OS game-controller endpoint, resolved only when a device connects.
use gilrs::Gamepad;
use sha2::{Digest, Sha256};

fn key(source: &str, value: &[u8]) -> String {
    // OS identifiers can contain a serial number, Bluetooth address or USB topology.
    format!("{source}:{}", hex::encode(Sha256::digest(value)))
}

#[cfg(any(windows, target_os = "macos", test))]
fn only<T>(mut candidates: impl Iterator<Item = T>) -> Option<T> {
    let candidate = candidates.next()?;
    candidates.next().is_none().then_some(candidate)
}

#[cfg(windows)]
pub(super) fn gilrs_id(pad: &Gamepad<'_>) -> Option<String> {
    use windows::Gaming::Input::{Gamepad as WgiGamepad, RawGameController};
    let (vendor, product) = (pad.vendor_id()?, pad.product_id()?);
    let devices = RawGameController::RawGameControllers().ok()?;
    let mut matches = Vec::new();
    // gilrs keeps the RawGameController private. Its index can be reused and independent WGI
    // enumerations need not have the same order. Leave indistinguishable devices unassigned.
    for index in 0..devices.Size().ok()? {
        // The WinRT vector iterator can crash under Steam Input; gilrs also uses GetAt.
        let device = devices.GetAt(index).ok()?;
        if device.HardwareVendorId().ok()? == vendor
            && device.HardwareProductId().ok()? == product
            && device.DisplayName().ok()?.to_string_lossy() == pad.os_name()
            && WgiGamepad::FromGameController(&device).is_ok() == (pad.uuid() == [0; 16])
        {
            matches.push(device);
        }
    }
    let id = only(matches.into_iter())?.NonRoamableId().ok()?;
    (!id.is_empty()).then(|| key("wgi", id.to_string_lossy().as_bytes()))
}

#[cfg(windows)]
pub(super) fn puck_id(path: &[u16]) -> Option<String> {
    let path = path.split(|&ch| ch == 0).next()?;
    let path = String::from_utf16(path).ok()?.to_ascii_lowercase();
    // Each HID interface is one radio slot, including when a different pad is paired to it.
    (!path.is_empty()).then(|| key("puck", path.as_bytes()))
}

#[cfg(target_os = "linux")]
pub(super) fn gilrs_id(pad: &Gamepad<'_>) -> Option<String> {
    use gilrs::LinuxGamepadExt;
    use std::{fs, os::unix::ffi::OsStrExt, path::Path};
    let device = fs::canonicalize(pad.devpath()).ok()?;
    // Prefer the actual OS endpoint. Some USB pads reuse a placeholder hardware serial.
    // udev can publish two links for a USB port, so choose deterministically.
    if let Some(link) = fs::read_dir("/dev/input/by-path").ok().and_then(|entries| {
        entries
            .flatten()
            .filter(|entry| fs::canonicalize(entry.path()).ok().as_ref() == Some(&device))
            .map(|entry| entry.file_name())
            .min()
    }) {
        return Some(key("evdev-path", link.as_bytes()));
    }
    // Bluetooth generally has no by-path link, but the input device supplies its address
    // in uniq. An eventN number and a model-only by-id link can both point to a different pad.
    let node = device.file_name()?;
    let uniq_path = Path::new("/sys/class/input").join(node).join("device/uniq");
    let uniq = fs::read_to_string(uniq_path).ok()?;
    let uniq = uniq.trim();
    if !uniq.chars().any(|c| c.is_alphanumeric() && c != '0') {
        return None;
    }
    let value = format!("{:04x}:{:04x}:{uniq}", pad.vendor_id()?, pad.product_id()?);
    Some(key("evdev-uniq", value.as_bytes()))
}

#[cfg(target_os = "macos")]
pub(super) fn gilrs_id(pad: &Gamepad<'_>) -> Option<String> {
    use objc2_core_foundation::{CFDictionary, CFNumber, CFString};
    use objc2_io_kit::{
        kHIDPage_GenericDesktop, kHIDUsage_GD_GamePad, kHIDUsage_GD_Joystick,
        kHIDUsage_GD_MultiAxisController, IOHIDDevice, IOHIDManager,
    };

    let (vendor, product) = (pad.vendor_id()?, pad.product_id()?);
    let vendor_key = CFString::from_static_str("VendorID");
    let product_key = CFString::from_static_str("ProductID");
    let vendor_num = CFNumber::new_i32(i32::from(vendor));
    let product_num = CFNumber::new_i32(i32::from(product));
    let matching = CFDictionary::from_slices(
        &[&*vendor_key, &*product_key],
        &[&*vendor_num, &*product_num],
    );
    let manager = IOHIDManager::new(None, 0);
    // Matching enumerates registry metadata. No second HID input stream is opened.
    unsafe { manager.set_device_matching(Some(matching.as_opaque())) };
    let devices = manager.devices()?;
    let mut pointers = vec![std::ptr::null(); usize::try_from(devices.count()).ok()?];
    // IOHIDManagerCopyDevices returns a retained, immutable CFSet of IOHIDDeviceRefs.
    unsafe { devices.values(pointers.as_mut_ptr()) };
    let name = CFString::from_static_str("Product");
    let device = only(pointers.into_iter().filter_map(|ptr| {
        let device = unsafe { ptr.cast::<IOHIDDevice>().as_ref()? };
        let usage = [
            kHIDUsage_GD_GamePad,
            kHIDUsage_GD_Joystick,
            kHIDUsage_GD_MultiAxisController,
        ]
        .iter()
        .any(|&u| device.conforms_to(kHIDPage_GenericDesktop, u));
        let matches = usage
            && device
                .property(&name)
                .and_then(|v| v.downcast::<CFString>().ok())
                .is_some_and(|n| n.to_string() == pad.os_name());
        matches.then_some(device)
    }))?;
    let serial = device
        .property(&CFString::from_static_str("SerialNumber"))
        .and_then(|v| v.downcast::<CFString>().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    if let Some(serial) = serial {
        return Some(key(
            "iohid-serial",
            format!("{vendor:04x}:{product:04x}:{serial}").as_bytes(),
        ));
    }
    let location = device
        .property(&CFString::from_static_str("LocationID"))?
        .downcast::<CFNumber>()
        .ok()?
        .as_i32()?;
    (location != 0).then(|| {
        key(
            "iohid-port",
            format!("{vendor:04x}:{product:04x}:{location}").as_bytes(),
        )
    })
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn gilrs_id(_: &Gamepad<'_>) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_key_keeps_its_persisted_format() {
        assert_eq!(
            key("device", b"abc"),
            "device:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn ambiguous_model_matches_do_not_get_a_device_key() {
        assert_eq!(only([10, 11].into_iter()), None);
        assert_eq!(only([10].into_iter()), Some(10));
        assert_eq!(only(std::iter::empty::<u8>()), None);
    }

    #[test]
    #[cfg(windows)]
    fn puck_keys_distinguish_radio_interfaces_and_ignore_win32_path_case() {
        let id = |s: &str| puck_id(&s.encode_utf16().collect::<Vec<_>>());
        assert_eq!(id("HID#MI_02#path\0\0"), id("hid#mi_02#PATH"));
        assert_ne!(id("HID#MI_02#path"), id("HID#MI_03#path"));
        assert_eq!(id(""), None);
    }

    #[test]
    #[ignore = "requires at least one desktop controller with a resolvable system endpoint"]
    fn keys_survive_recreating_the_gamepad_backend() {
        use std::collections::BTreeMap;
        use std::time::{Duration, Instant};

        fn sample() -> BTreeMap<String, String> {
            let mut service = crate::Gamepads::new().unwrap();
            let until = Instant::now() + Duration::from_secs(1);
            while Instant::now() < until {
                let _ = service.sample();
                std::thread::sleep(Duration::from_millis(8));
            }
            let mut endpoints = BTreeMap::new();
            for (_, pad) in service.pads() {
                eprintln!(
                    "{} ({:04x}:{:04x}), system key available: {}",
                    pad.name,
                    pad.vendor,
                    pad.product,
                    pad.system_id.is_some()
                );
                if let Some(id) = &pad.system_id {
                    assert!(
                        endpoints.insert(id.clone(), pad.name.clone()).is_none(),
                        "two live controllers share a system key"
                    );
                }
            }
            endpoints
        }

        let first = sample();
        assert!(!first.is_empty(), "no resolvable gamepad connected");
        let second = sample();
        assert!(
            first == second,
            "system keys or connected controllers changed"
        );
    }
}
