const ELITE_SERIES_2: (u16, u16) = (0x045e, 0x0b00);

// USB, classic Bluetooth and BLE product IDs of the same Xbox Elite Series 2.
// https://github.com/libsdl-org/SDL/blob/main/src/joystick/controller_list.h
pub(crate) fn model_key(vendor: u16, product: u16) -> (u16, u16) {
    match (vendor, product) {
        (0x045e, 0x0b00 | 0x0b05 | 0x0b22) => ELITE_SERIES_2,
        key => key,
    }
}

/// Both names and hardware IDs come from the same gilrs gamepad. On Windows the IDs and
/// OS name refer to its RawGameController, including when WGI supplies a standard mapping.
pub(super) fn resolve<'a>(
    vendor: u16,
    product: u16,
    os_name: &'a str,
    mapping_name: Option<&'a str>,
) -> (&'a str, &'static str) {
    let mapped = mapping_name.filter(|s| !s.is_empty());
    let name = mapped.unwrap_or(os_name);
    let os = os_name.to_ascii_lowercase();
    let mapped_lower = mapped.unwrap_or("").to_ascii_lowercase();
    if os.contains("steam virtual") {
        return (os_name, "generic");
    }
    if mapped_lower.contains("steam virtual") {
        return (name, "generic");
    }

    // A receiver or the shared XInput driver PID alone carries no controller model.
    match model_key(vendor, product) {
        (0x28de, 0x11fc | 0x11ff) => return ("Steam Virtual Gamepad", "generic"),
        (0x045e, 0x02e3) => return ("Xbox Elite Wireless Controller", "xbox"),
        ELITE_SERIES_2 => return ("Xbox Elite Wireless Controller Series 2", "xbox"),
        _ => {}
    }
    if matches!(vendor, 0 | 0x045e) && os.contains("xbox") && os.contains("elite") {
        return (os_name, "xbox");
    }

    let family = match vendor {
        0x045e => "xbox",
        0x054c => "playstation",
        0x057e => "nintendo",
        0x28de => "steam",
        _ => [os.as_str(), mapped_lower.as_str()]
            .into_iter()
            .find_map(family_from_name)
            .unwrap_or("generic"),
    };
    (name, family)
}

fn family_from_name(name: &str) -> Option<&'static str> {
    if name.contains("xbox") || name.contains("x-box") {
        Some("xbox")
    } else if name.contains("dualshock")
        || name.contains("dualsense")
        || name.contains("playstation")
    {
        Some("playstation")
    } else if name.contains("steam controller") || name.contains("steam deck") {
        Some("steam")
    } else if name.contains("nintendo") || name.contains("joy-con") {
        Some("nintendo")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn elite_product_ids_identify_the_controller_across_transports() {
        let hid = "HID-compliant game controller";
        assert_eq!(
            resolve(0x045e, 0x02e3, hid, None),
            ("Xbox Elite Wireless Controller", "xbox")
        );
        for product in [0x0b00, 0x0b05, 0x0b22] {
            assert_eq!(
                resolve(0x045e, product, hid, Some("Xbox Wireless Controller")),
                ("Xbox Elite Wireless Controller Series 2", "xbox")
            );
        }
        for product in [0x028e, 0x02ff, 0x02e6, 0x02fe] {
            assert_eq!(resolve(0x045e, product, hid, None).0, hid);
        }
        let elite = "Xbox Elite Wireless Controller Series 2";
        assert_eq!(
            resolve(0x045e, 0x02ff, elite, Some("Xbox Controller")),
            (elite, "xbox")
        );
        assert_eq!(resolve(0, 0, hid, None), (hid, "generic"));
        assert_eq!(resolve(0x1234, 0x0b22, hid, None), (hid, "generic"));
    }

    #[test]
    fn steam_virtual_identity_takes_precedence_over_a_mapping_or_emulated_ids() {
        let virtual_name = "Steam Virtual Gamepad";
        assert_eq!(
            resolve(0x045e, 0x0b22, virtual_name, Some("Xbox Elite")),
            (virtual_name, "generic")
        );
        assert_eq!(
            resolve(0x045e, 0x028e, "Gamepad", Some(virtual_name)),
            (virtual_name, "generic")
        );
        for product in [0x11fc, 0x11ff] {
            assert_eq!(
                resolve(0x28de, product, "Gamepad", Some("Xbox Controller")),
                (virtual_name, "generic")
            );
        }
    }

    #[test]
    fn other_families_use_the_reported_identity() {
        assert_eq!(
            resolve(
                0x054c,
                0x09cc,
                "Wireless Controller",
                Some("PS4 Controller")
            ),
            ("PS4 Controller", "playstation")
        );
        assert_eq!(resolve(0, 0, "Xbox Series Controller", None).1, "xbox");
        assert_eq!(
            resolve(0x28de, 0x1102, "Valve Steam Controller", None).1,
            "steam"
        );
        assert_eq!(resolve(0, 0, "USB Controller", None).1, "generic");
    }
}
