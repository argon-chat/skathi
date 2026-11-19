use crate::EventPayload;
use gpui::{Keystroke, NativeKey, NativeKeyCode};
use keycode::{KeyMap, KeyMapping};

pub(super) struct Keyboard {
    key: String,
    code: String,
    location: u8,
    repeat: bool,
    modifiers: gpui::Modifiers,
}

impl Keyboard {
    pub(super) fn new(stroke: &Keystroke, native: Option<&NativeKey>, repeat: bool) -> Self {
        let code = native
            .and_then(|n| match n.code {
                NativeKeyCode::Windows(0)
                | NativeKeyCode::Evdev(0)
                | NativeKeyCode::Mac(0xffff) => None,
                NativeKeyCode::Windows(v) => Some(KeyMapping::Win(v)),
                NativeKeyCode::Evdev(v) => Some(KeyMapping::Evdev(v)),
                NativeKeyCode::Mac(v) => Some(KeyMapping::Mac(v)),
            })
            .and_then(|mapping| KeyMap::from_key_mapping(mapping).ok()?.code)
            .map(|code| code.to_string())
            .unwrap_or_else(|| {
                if native.is_some() {
                    "Unidentified".into()
                } else {
                    String::new()
                }
            });
        let location = match code.as_str() {
            "AltLeft" | "ControlLeft" | "ShiftLeft" | "MetaLeft" => 1,
            "AltRight" | "ControlRight" | "ShiftRight" | "MetaRight" => 2,
            code if code.starts_with("Numpad") => 3,
            _ => 0,
        };
        let key =
            native.and_then(|n| n.key.clone()).unwrap_or_else(|| {
                if let Some(text) = stroke.key_char.as_ref().filter(|s| {
                    native.is_none() && !s.is_empty() && !s.chars().any(char::is_control)
                }) {
                    return text.clone();
                }
                dom_key(&stroke.key)
            });
        Self {
            key,
            code,
            location,
            repeat,
            modifiers: native.map_or(stroke.modifiers, |n| n.modifiers),
        }
    }

    pub(super) fn dispatch(
        &self,
        target: u32,
        event: u32,
        window: &mut gpui::Window,
        cx: &mut gpui::App,
    ) -> bool {
        crate::render::js_wave_app(window, cx, || {
            let code = sk_runtime::JsRuntime::try_with(|rt| rt.ctx().string(&self.code));
            crate::dispatch_event_with(target, event, &self.payload(), code)
        })
    }

    fn payload(&self) -> EventPayload<'_> {
        EventPayload {
            a: self.location.into(),
            b: u8::from(self.repeat).into(),
            d: crate::render::mods_bits(&self.modifiers),
            key: Some(&self.key),
            ..Default::default()
        }
    }
}

fn dom_key(k: &str) -> String {
    match k {
        "enter" => "Enter".into(),
        "escape" => "Escape".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "tab" => "Tab".into(),
        "space" => " ".into(),
        "up" => "ArrowUp".into(),
        "down" => "ArrowDown".into(),
        "left" => "ArrowLeft".into(),
        "right" => "ArrowRight".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "insert" => "Insert".into(),
        "shift" => "Shift".into(),
        "control" => "Control".into(),
        "alt" => "Alt".into(),
        "platform" => "Meta".into(),
        "function" => "Fn".into(),
        "capslock" => "CapsLock".into(),
        "numlock" => "NumLock".into(),
        "scrolllock" => "ScrollLock".into(),
        "clear" => "Clear".into(),
        "pause" => "Pause".into(),
        "printscreen" => "PrintScreen".into(),
        "back" => "BrowserBack".into(),
        "forward" => "BrowserForward".into(),
        // The Menu key is VK_APPS, which gpui names "menu"; the DOM's name is "ContextMenu".
        "menu" => "ContextMenu".into(),
        f if f.len() > 1
            && f.starts_with('f')
            && f[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) =>
        {
            f.to_ascii_uppercase()
        }
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::NativeKeyCode::{Evdev, Mac, Windows};

    #[test]
    fn physical_positions_agree_across_native_keyboards() {
        for (win, mac, linux, code, location) in [
            (0x26, 37, 38, "KeyL", 0),
            (0x10, 12, 16, "KeyQ", 0),
            (0x02, 18, 2, "Digit1", 0),
            (0x4f, 83, 79, "Numpad1", 3),
            (0x1c, 36, 28, "Enter", 0),
            (0xe01c, 76, 96, "NumpadEnter", 3),
            (0x39, 49, 57, "Space", 0),
            (0x2a, 56, 42, "ShiftLeft", 1),
            (0x36, 60, 54, "ShiftRight", 2),
            (0x1d, 59, 29, "ControlLeft", 1),
            (0xe01d, 62, 97, "ControlRight", 2),
            (0xe038, 61, 100, "AltRight", 2),
            (0x38, 58, 56, "AltLeft", 1),
            (0xe05b, 55, 125, "MetaLeft", 1),
            (0xe05c, 54, 126, "MetaRight", 2),
            (0xe045, 71, 69, "NumLock", 0),
        ] {
            for native in [Windows(win), Mac(mac), Evdev(linux)] {
                let original = NativeKey {
                    code: native,
                    key: None,
                    modifiers: Default::default(),
                };
                let event = Keyboard::new(&Keystroke::default(), Some(&original), false);
                assert_eq!(
                    (event.code.as_str(), event.location),
                    (code, location),
                    "{native:?}"
                );
            }
        }
    }

    #[test]
    fn layout_text_modifiers_and_repeat_are_independent_of_the_shortcut() {
        for (stroke, physical, text, shift, code) in [
            ("l", Mac(37), "L", true, "KeyL"),
            ("!", Windows(2), "!", true, "Digit1"),
            ("a", Evdev(16), "a", false, "KeyQ"),
            ("&", Mac(18), "&", false, "Digit1"),
            ("1", Windows(0x4f), "1", false, "Numpad1"),
            ("q", Evdev(16), "й", false, "KeyQ"),
        ] {
            let original = NativeKey {
                code: physical,
                key: Some(text.into()),
                modifiers: gpui::Modifiers {
                    shift,
                    ..Default::default()
                },
            };
            for repeat in [false, true] {
                let key =
                    Keyboard::new(&Keystroke::parse(stroke).unwrap(), Some(&original), repeat);
                let event = key.payload();
                assert_eq!(event.key, Some(text));
                assert_eq!(key.code, code);
                assert_eq!(event.d, if shift { 2.0 } else { 0.0 });
                assert_eq!(event.b, u8::from(repeat) as f64);
            }
        }
        let stroke = Keystroke::parse("q").unwrap();
        for code in [Windows(0), Evdev(0), Mac(0xffff), Mac(0xfeed)] {
            let original = NativeKey {
                code,
                key: None,
                modifiers: Default::default(),
            };
            assert_eq!(
                Keyboard::new(&stroke, Some(&original), false).code,
                "Unidentified"
            );
        }
        assert_eq!(Keyboard::new(&stroke, None, false).code, "");
    }
}
