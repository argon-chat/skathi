use super::{Binding, Button, Chord, EITHER, Fired, POSITION};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Output {
    pub events: Vec<Fired>,
    pub capture: Option<Result<Chord, String>>,
    pub suppress: bool,
}
struct Capture {
    ever: Vec<Button>,
    down: BTreeSet<Button>,
    deadline: Instant,
}
#[derive(Default)]
pub(super) struct Engine {
    bindings: Vec<Binding>,
    pressed: BTreeSet<Button>,
    positions: BTreeMap<Button, u16>,
    active: BTreeSet<String>,
    suppressed: BTreeSet<Button>,
    last_fired: BTreeMap<String, Instant>,
    capture: Option<Capture>,
}
impl Engine {
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }
    pub fn pressed(&self) -> Vec<Button> {
        self.pressed.iter().copied().collect()
    }
    pub fn pollable_pressed(&self) -> Vec<Button> {
        // Consuming a low-level event prevents the OS async key state from
        // changing. GetAsyncKeyState cannot reconcile those physical holds.
        self.pressed.difference(&self.suppressed).copied().collect()
    }
    pub fn watched(&self) -> BTreeSet<Button> {
        self.pressed
            .iter()
            .copied()
            .chain(
                self.bindings
                    .iter()
                    .flat_map(|b| b.chord.buttons.iter().copied()),
            )
            .collect()
    }
    /// Whether mouse buttons can matter: a binding uses one, or a capture may
    /// record one. Held mouse buttons never block a keyboard chord.
    pub fn wants_mouse(&self) -> bool {
        self.capture.is_some()
            || self
                .bindings
                .iter()
                .any(|b| b.chord.buttons.iter().any(|button| button.device == 1))
    }
    /// Drops the held state of a device whose events are no longer observed.
    pub fn forget_device(&mut self, device: u8) {
        self.pressed.retain(|b| b.device != device);
        self.suppressed.retain(|b| b.device != device);
    }
    pub fn seed(&mut self, buttons: impl IntoIterator<Item = Button>) {
        self.pressed.extend(buttons);
    }
    pub fn set_bindings(&mut self, bindings: Vec<Binding>) -> (bool, Output) {
        if self.bindings == bindings {
            return (false, Output::default());
        }
        let mut output = self.release_active();
        self.abort("cancelled", &mut output);
        self.bindings = bindings;
        self.last_fired.clear();
        (true, output)
    }
    pub fn begin_capture(&mut self, now: Instant, held_modifiers: Vec<Button>) -> Output {
        let mut output = self.release_active();
        self.abort("superseded", &mut output);
        self.pressed.extend(held_modifiers);
        let ever: Vec<_> = self
            .pressed
            .iter()
            .copied()
            .filter(Button::modifier)
            .collect();
        self.capture = Some(Capture {
            down: ever.iter().copied().collect(),
            ever,
            deadline: now + Duration::from_secs(10),
        });
        output
    }
    pub fn cancel_capture(&mut self) -> Output {
        let mut output = Output::default();
        self.abort("cancelled", &mut output);
        output
    }
    fn abort(&mut self, reason: &str, output: &mut Output) {
        if self.capture.take().is_some() {
            output.capture = Some(Err(format!("hotkey capture {reason}")));
        }
    }
    fn finish(&mut self, output: &mut Output) {
        if let Some(capture) = self.capture.take() {
            output.capture = Some(Ok(Chord {
                buttons: capture.ever,
            }));
        }
    }
    pub fn tick(&mut self, now: Instant) -> Output {
        let mut output = Output::default();
        if self.capture.as_ref().is_some_and(|c| now >= c.deadline) {
            if self.capture.as_ref().is_some_and(|c| !c.ever.is_empty()) {
                self.finish(&mut output);
            } else {
                self.abort("timeout", &mut output);
            }
        }
        output
    }
    #[cfg(test)]
    pub fn input(&mut self, button: Button, down: bool, now: Instant) -> Output {
        self.input_at(button, 0, down, now)
    }
    pub fn input_at(&mut self, mut button: Button, scan: u16, down: bool, now: Instant) -> Output {
        let mut output = Output::default();
        if scan != 0 {
            // The VK can change between down and up when the focused application changes layouts.
            if let Some((&held, _)) = self.positions.iter().find(|(_, pos)| **pos == scan) {
                button = held;
            } else if down {
                self.positions.insert(button, scan);
            }
        }
        let matches = |required: &Button, actual: &Button| {
            if required == actual {
                return true;
            }
            if required.device != 0 || actual.device != 0 {
                return false;
            }
            match required.code & 0xffff0000 {
                POSITION => self.positions.get(actual) == Some(&(required.code as u16)),
                EITHER => match required.code & 0xffff {
                    0x10 => matches!(actual.code, 0xa0 | 0xa1),
                    0x11 => matches!(actual.code, 0xa2 | 0xa3),
                    0x12 => matches!(actual.code, 0xa4 | 0xa5),
                    0x5b => matches!(actual.code, 0x5b | 0x5c),
                    _ => false,
                },
                _ => false,
            }
        };
        if down {
            if !self.pressed.insert(button) {
                output.suppress = self.suppressed.contains(&button);
                return output;
            }
            if self.capture.is_some() {
                if button == Button::keyboard(0x1b) {
                    self.abort("cancelled", &mut output);
                    output.suppress = true;
                } else if !(button.device == 1 && button.code <= 2) {
                    let capture = self.capture.as_mut().unwrap();
                    if !capture.ever.contains(&button) && capture.ever.len() < 16 {
                        capture.ever.push(button);
                    }
                    if capture.ever.contains(&button) {
                        capture.down.insert(button);
                    }
                    output.suppress = true;
                }
            } else {
                let mut best = 0;
                let mut hits = Vec::new();
                for binding in &self.bindings {
                    let chord = &binding.chord.buttons;
                    if self.active.contains(&binding.id)
                        || !chord.iter().any(|b| matches(b, &button))
                    {
                        continue;
                    }
                    if !binding.chord.buttons.iter().all(|b| {
                        self.pressed.contains(b) || self.pressed.iter().any(|p| matches(b, p))
                    }) {
                        continue;
                    }
                    if self
                        .pressed
                        .iter()
                        .any(|b| b.modifier() && !chord.iter().any(|key| matches(key, b)))
                    {
                        continue;
                    }
                    let size = binding.chord.buttons.len();
                    if size > best {
                        best = size;
                        hits.clear();
                    }
                    if size == best {
                        hits.push(binding);
                    }
                }
                for binding in hits {
                    let cooldown = Duration::from_millis(binding.trigger_cooldown_ms.unwrap_or(0));
                    if self
                        .last_fired
                        .get(&binding.id)
                        .is_some_and(|last| now.saturating_duration_since(*last) < cooldown)
                    {
                        continue;
                    }
                    self.last_fired.insert(binding.id.clone(), now);
                    self.active.insert(binding.id.clone());
                    output.events.push(Fired::down(&binding.id));
                    output.suppress |= binding.suppress.unwrap_or(false);
                }
            }
            if output.suppress {
                self.suppressed.insert(button);
            }
        } else {
            output.suppress = self.suppressed.remove(&button);
            let was_pressed = self.pressed.remove(&button);
            let capture_done = self.capture.as_mut().is_some_and(|capture| {
                capture.down.remove(&button) && capture.down.is_empty() && !capture.ever.is_empty()
            });
            if was_pressed {
                for binding in &self.bindings {
                    if self.active.contains(&binding.id)
                        && binding.chord.buttons.iter().any(|key| {
                            matches(key, &button)
                                && !self.pressed.iter().any(|held| matches(key, held))
                        })
                        && self.active.remove(&binding.id)
                    {
                        output.events.push(Fired::up(&binding.id));
                    }
                }
            }
            self.positions.remove(&button);
            if capture_done {
                self.finish(&mut output);
            }
        }
        output
    }
    fn release_active(&mut self) -> Output {
        let events = std::mem::take(&mut self.active)
            .iter()
            .map(|id| Fired::cancel(id))
            .collect();
        Output {
            events,
            ..Default::default()
        }
    }
    pub fn reset(&mut self) -> Output {
        let mut output = self.release_active();
        self.pressed.clear();
        self.positions.clear();
        self.suppressed.clear();
        self.last_fired.clear();
        self.abort("cancelled", &mut output);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(vk: u32) -> Button {
        Button::keyboard(vk)
    }
    fn binding(id: &str, keys: &[u32]) -> Binding {
        Binding {
            id: id.into(),
            chord: Chord {
                buttons: keys.iter().map(|k| key(*k)).collect(),
            },
            suppress: None,
            trigger_cooldown_ms: None,
        }
    }
    #[test]
    fn physical_chords_follow_scans_and_accept_either_modifier_side() {
        let mut e = Engine::default();
        let now = Instant::now();
        e.set_bindings(vec![binding("find", &[EITHER | 0x11, POSITION | 0x10])]);
        e.input_at(key(0xa3), 0xe01d, true, now);
        assert!(e.input_at(key(0x51), 0x1e, true, now).events.is_empty());
        e.input_at(key(0x51), 0x1e, false, now);
        // AZERTY produces VK_A at the US Q position; the key-up can arrive following a layout switch.
        assert_eq!(
            e.input_at(key(0x41), 0x10, true, now).events,
            [Fired::down("find")]
        );
        assert!(e.input_at(key(0x41), 0x10, true, now).events.is_empty());
        e.input_at(key(0xa2), 0x1d, true, now);
        assert!(e.input_at(key(0xa3), 0xe01d, false, now).events.is_empty());
        assert_eq!(
            e.input_at(key(0x51), 0x10, false, now).events,
            [Fired::up("find")]
        );
        e.input_at(key(0xa0), 0x2a, true, now);
        assert!(e.input_at(key(0x51), 0x10, true, now).events.is_empty());
    }
    #[test]
    fn the_mouse_matters_only_to_mouse_bindings_and_captures() {
        let mut e = Engine::default();
        let now = Instant::now();
        let x1 = Button { device: 1, code: 4 };
        e.set_bindings(vec![binding("k", &[0x4b])]);
        assert!(!e.wants_mouse());
        // A held mouse button does not block a keyboard chord.
        e.input(Button { device: 1, code: 1 }, true, now);
        assert_eq!(e.input(key(0x4b), true, now).events, [Fired::down("k")]);
        e.reset();
        let mut mouse = binding("x1", &[]);
        mouse.chord.buttons.push(x1);
        e.set_bindings(vec![mouse]);
        assert!(e.wants_mouse());
        e.input(x1, true, now);
        e.forget_device(1);
        assert!(e.pressed().is_empty());
        e.set_bindings(vec![binding("k", &[0x4b])]);
        e.begin_capture(now, Vec::new());
        assert!(e.wants_mouse());
        e.cancel_capture();
        assert!(!e.wants_mouse());
    }
    #[test]
    fn longest_chord_extra_modifiers_walking_keys_and_autorepeat() {
        let mut e = Engine::default();
        let now = Instant::now();
        e.set_bindings(vec![
            binding("m", &[0x4d]),
            binding("ctrl-m", &[0xa2, 0x4d]),
        ]);
        e.input(key(0xa2), true, now);
        let hit = e.input(key(0x4d), true, now);
        assert_eq!(hit.events, [Fired::down("ctrl-m")]);
        assert!(e.input(key(0x4d), true, now).events.is_empty());
        assert_eq!(e.input(key(0xa2), false, now).events, [Fired::up("ctrl-m")]);
        e.reset();
        e.input(key(0x57), true, now);
        assert_eq!(e.input(key(0x4d), true, now).events, [Fired::down("m")]);
        e.reset();
        e.input(key(0xa3), true, now);
        assert!(
            e.input(key(0x4d), true, now).events.is_empty(),
            "right Ctrl does not satisfy left Ctrl or plain M"
        );
    }
    #[test]
    fn suppression_balances_only_keys_whose_down_was_consumed() {
        let mut e = Engine::default();
        let now = Instant::now();
        let mut b = binding("hold", &[0xa2, 0x4d]);
        b.suppress = Some(true);
        e.set_bindings(vec![b]);
        assert!(!e.input(key(0xa2), true, now).suppress);
        assert!(e.input(key(0x4d), true, now).suppress);
        assert_eq!(e.pollable_pressed(), [key(0xa2)]);
        assert!(
            !e.input(key(0xa2), false, now).suppress,
            "do not leave Ctrl stuck in the foreground app"
        );
        assert!(e.input(key(0x4d), false, now).suppress);
        assert!(e.pollable_pressed().is_empty());
    }
    #[test]
    fn capture_includes_preheld_modifiers_ignores_primary_clicks_and_finishes_on_release() {
        let mut e = Engine::default();
        let now = Instant::now();
        e.begin_capture(now, vec![key(0xa2)]);
        assert!(
            e.input(Button { device: 1, code: 1 }, true, now)
                .capture
                .is_none()
        );
        e.input(Button { device: 1, code: 1 }, false, now);
        e.input(key(0x4b), true, now);
        e.input(key(0x4b), false, now);
        let result = e.input(key(0xa2), false, now).capture.unwrap().unwrap();
        assert_eq!(result.buttons, [key(0xa2), key(0x4b)]);
    }
    #[test]
    fn capture_cancel_timeout_supersede_and_held_deadline() {
        let mut e = Engine::default();
        let now = Instant::now();
        e.begin_capture(now, vec![]);
        assert_eq!(
            e.begin_capture(now, vec![]).capture,
            Some(Err("hotkey capture superseded".into()))
        );
        assert_eq!(
            e.tick(now + Duration::from_secs(10)).capture,
            Some(Err("hotkey capture timeout".into()))
        );
        e.begin_capture(now, vec![]);
        e.input(key(0x70), true, now);
        assert_eq!(
            e.tick(now + Duration::from_secs(10))
                .capture
                .unwrap()
                .unwrap()
                .buttons,
            [key(0x70)]
        );
        e.begin_capture(now, vec![]);
        assert_eq!(
            e.input(key(0x1b), true, now).capture,
            Some(Err("hotkey capture cancelled".into()))
        );
    }
    #[test]
    fn cooldown_applies_to_down_edges_and_every_hold_gets_an_up() {
        let mut e = Engine::default();
        let now = Instant::now();
        let mut b = binding("hold", &[0x70]);
        b.trigger_cooldown_ms = Some(100);
        e.set_bindings(vec![b]);
        assert_eq!(e.input(key(0x70), true, now).events, [Fired::down("hold")]);
        assert_eq!(e.input(key(0x70), false, now).events, [Fired::up("hold")]);
        assert!(
            e.input(key(0x70), true, now + Duration::from_millis(50))
                .events
                .is_empty()
        );
        e.input(key(0x70), false, now);
        assert_eq!(
            e.input(key(0x70), true, now + Duration::from_millis(100))
                .events,
            [Fired::down("hold")]
        );
        assert_eq!(e.reset().events, [Fired::cancel("hold")]);
    }
    #[test]
    fn reconfiguration_and_capture_start_release_active_push_to_talk() {
        let mut e = Engine::default();
        let now = Instant::now();
        e.set_bindings(vec![binding("ptt", &[0x70])]);
        e.input(key(0x70), true, now);
        assert_eq!(
            e.set_bindings(vec![binding("ptt", &[0x71])]).1.events,
            [Fired::cancel("ptt")]
        );
        e.input(key(0x71), true, now);
        assert_eq!(e.begin_capture(now, vec![]).events, [Fired::cancel("ptt")]);
        assert!(e.input(key(0x71), true, now).events.is_empty());
    }
}
