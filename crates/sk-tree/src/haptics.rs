//! CSS feedback follows the control and window that received the interaction.
use crate::{
    tree::{self, Tree},
    windows, EventPayload,
};
use gpui::{App, HapticPattern as Wave, HapticTarget, Task, Window};
use sk_css::haptics::{Binding, Cue, Effect, Haptics, Pattern, Target, Texture, Trigger};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

type Key = (u32, Trigger, Target);

thread_local! {
    static NATIVE_CLICK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn dispatch(id: u32, event: u32, payload: &EventPayload) -> bool {
    // Mouse releases can dispatch off the gpui input stack. The checkbox commits inside this click.
    if event != tree::EV_CLICK {
        return crate::dispatch_event(id, event, payload);
    }
    NATIVE_CLICK.with(|active| {
        struct Restore<'a>(&'a std::cell::Cell<bool>, bool);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let _restore = Restore(active, active.replace(true));
        crate::dispatch_event(id, event, payload)
    })
}

pub(crate) fn in_native_click() -> bool {
    NATIVE_CLICK.with(std::cell::Cell::get)
}

#[derive(Default)]
pub(crate) struct State {
    last: HashMap<Key, Instant>,
    sequences: HashMap<Key, Task<()>>,
    stroke: Option<(u32, Texture, f32)>,
    pub(crate) snaps: Vec<(u32, HapticTarget)>,
}
impl State {
    pub(crate) fn gc(&mut self, live: &impl Fn(u32) -> bool) {
        self.last.retain(|k, _| live(k.0));
        self.sequences.retain(|k, _| live(k.0));
        self.snaps.retain(|&(id, _)| live(id));
    }
}

fn waves(cue: Cue) -> &'static [Wave] {
    use Wave::*;
    match cue {
        Cue::Tick => &[LevelChange, Click, Generic],
        Cue::Hint => &[Hover, Generic],
        Cue::Align => &[Alignment, Generic],
        Cue::Edge => &[Collide, LevelChange, Generic],
        Cue::Success => &[Success, Generic],
        Cue::Error => &[Error, Generic],
        Cue::None => &[],
    }
}
pub(crate) fn input_target(w: &Window) -> HapticTarget {
    if w.last_input_was_keyboard() {
        HapticTarget::Current
    } else if w.vn_pen().is_some() {
        HapticTarget::Pen
    } else {
        HapticTarget::Mouse
    }
}
fn target(rule: Target, input: HapticTarget) -> HapticTarget {
    match rule {
        Target::Auto => input,
        Target::Mouse => HapticTarget::Mouse,
        Target::Pen => HapticTarget::Pen,
    }
}
fn key_target(t: HapticTarget) -> Target {
    match t {
        HapticTarget::Current => Target::Auto,
        HapticTarget::Mouse => Target::Mouse,
        HapticTarget::Pen => Target::Pen,
    }
}
pub(crate) fn mask(node: &tree::Node) -> u64 {
    let Some(h) = node
        .computed
        .as_ref()
        .and_then(|cs| cs.extras.haptics.as_deref())
    else {
        return 0;
    };
    let mut bits = 0;
    if h.texture.is_some_and(|t| t != Texture::None) {
        bits |= (1 << tree::EV_MOUSEDOWN) | (1 << tree::EV_MOUSEUP) | (1 << tree::EV_MOUSELEAVE);
    }
    for t in h.triggers.iter().flatten() {
        bits |= match t {
            Trigger::Press => 1 << tree::EV_MOUSEDOWN,
            Trigger::Activate => 1 << tree::EV_CLICK,
            Trigger::Hover => 1 << tree::EV_MOUSEENTER,
            _ => 0,
        };
    }
    bits
}

fn owner(
    t: &Tree,
    mut id: u32,
    root: u32,
    exact: bool,
    matches: impl Fn(&Haptics) -> bool,
) -> Option<(u32, Rc<Haptics>)> {
    let root = if root == 0 { t.root } else { root };
    let origin = id;
    loop {
        let node = t.node(id)?;
        if let Some(h) = node
            .computed
            .as_ref()
            .and_then(|c| c.extras.haptics.as_ref())
        {
            if matches(h) {
                crate::pointer::enabled_path(t, origin, root)?;
                return Some((id, h.clone()));
            }
            if h.triggers.as_ref().is_some_and(Vec::is_empty) {
                return None;
            }
        }
        if exact || id == root || node.parent == id {
            return None;
        }
        id = node.parent;
    }
}
pub fn trigger(
    id: u32,
    trigger: Trigger,
    source: HapticTarget,
    w: &mut Window,
    cx: &mut App,
) -> bool {
    if windows::window_of_node(id) != windows::key(w) {
        return false;
    }
    let root = windows::root_of(windows::key(w));
    let Some((id, h)) = tree::read_tree(|t| {
        owner(t, id, root, trigger == Trigger::Hover, |h| {
            h.binding(trigger).is_some()
        })
    }) else {
        return false;
    };
    let b = h.binding(trigger).unwrap();
    let device = target(b.target, source);
    let key = (id, trigger, key_target(device));
    let now = Instant::now();
    if b.intensity == 0. || matches!(b.effect, Effect::Cue(Cue::None)) {
        return false;
    }
    if !b.interval.is_zero()
        && windows::here(w, |s| {
            if s.haptics
                .last
                .get(&key)
                .is_some_and(|last| now.duration_since(*last) < b.interval)
            {
                return true;
            }
            s.haptics.last.insert(key, now);
            false
        })
    {
        return false;
    }
    windows::here(w, |s| s.haptics.sequences.remove(&key));
    let accepted = match b.effect {
        Effect::Cue(cue) => w.vn_haptic_at_strength(waves(*cue), device, b.intensity),
        Effect::Pattern(name) => {
            let env = windows::env_of(windows::key(w)).unwrap_or_else(tree::env);
            tree::with_engine(|engine| engine.and_then(|e| e.haptic_pattern(name, &env)))
                .is_some_and(|pat| sequence(key, &b, &pat, device, w, cx))
        }
    };
    if crate::log::is_enabled() {
        crate::log::record(
            "haptic",
            format!(
                "{} node={} {:?} intensity={} accepted={accepted}",
                trigger.name(),
                id,
                b.effect,
                b.intensity
            ),
        );
    }
    accepted
}

pub(crate) fn has_boundary(id: u32, w: &Window) -> bool {
    let root = windows::root_of(windows::key(w));
    tree::read_tree(|t| {
        owner(t, id, root, false, |h| {
            h.binding(Trigger::Boundary).is_some()
        })
        .is_some()
    })
}

fn sequence(
    key: Key,
    binding: &Binding,
    pat: &Pattern,
    device: HapticTarget,
    w: &mut Window,
    cx: &mut App,
) -> bool {
    let mut pulses = pat.scaled_pulses(binding.intensity).peekable();
    if pulses.peek().is_none() {
        return false;
    }
    let Some(play) = w.vn_prepare_haptic_pulse(device) else {
        return w.vn_haptic_at_strength(waves(pat.fallback), device, binding.intensity);
    };
    let start = Instant::now();
    let mut immediate = false;
    let mut delayed = Vec::new();
    for (offset, strength) in pulses {
        if offset.is_zero() {
            immediate = play(strength);
        } else if let Some(due) = start.checked_add(offset) {
            delayed.push((due, strength));
        }
    }
    if delayed.is_empty() {
        return immediate;
    }
    let task = w.spawn(cx, async move |cx| {
        for (due, strength) in delayed {
            cx.background_executor()
                .timer(due.saturating_duration_since(Instant::now()))
                .await;
            let sent = cx.update(|w, _| {
                w.is_window_active() && tree::read_tree(|t| t.connected(key.0)) && play(strength)
            });
            if !matches!(sent, Ok(true)) {
                break;
            }
        }
    });
    windows::here(w, |s| s.haptics.sequences.insert(key, task));
    true
}

fn texture(h: &Haptics) -> Option<(Texture, f32)> {
    let texture = h.texture?;
    let strength = h.texture_intensity.unwrap_or(1.);
    (h.texture_target() != Target::Mouse && texture != Texture::None && strength > 0.)
        .then_some((texture, strength))
}
fn play_texture(id: u32, texture: Texture, strength: f32, w: &mut Window) {
    let wave = match texture {
        Texture::Ink => Wave::InkContinuous,
        Texture::Pencil => Wave::PencilContinuous,
        Texture::Brush => Wave::BrushContinuous,
        Texture::Eraser => Wave::EraserContinuous,
        Texture::None => return,
    };
    if w.vn_haptic_at_strength(&[wave], HapticTarget::Pen, strength) {
        windows::here(w, |s| s.haptics.stroke = Some((id, texture, strength)));
    }
}
fn start_texture(id: u32, w: &mut Window) {
    if !w.vn_pen().is_some_and(|p| p.in_contact) {
        return;
    }
    end_texture(w);
    let root = windows::root_of(windows::key(w));
    let Some((owner, h)) = tree::read_tree(|t| owner(t, id, root, false, |h| h.texture.is_some()))
    else {
        return;
    };
    if let Some((texture, strength)) = texture(&h) {
        play_texture(owner, texture, strength, w);
    }
}
fn end_texture(w: &mut Window) {
    if windows::here(w, |s| s.haptics.stroke.take()).is_some() {
        w.vn_stop_haptic(HapticTarget::Pen);
    }
}
pub(crate) fn blur(w: &mut Window) {
    windows::here(w, |s| {
        s.haptics.sequences.clear();
        s.haptics.snaps.clear();
    });
    end_texture(w);
}
pub(crate) fn tick(w: &mut Window, cx: &mut App) {
    let (snaps, stroke) = windows::here(w, |s| {
        (std::mem::take(&mut s.haptics.snaps), s.haptics.stroke)
    });
    if let Some((id, previous, strength)) = stroke {
        let current = tree::read_tree(|t| {
            if !t.connected(id) {
                return None;
            }
            texture(t.node(id)?.computed.as_ref()?.extras.haptics.as_deref()?)
        });
        if current != Some((previous, strength)) {
            end_texture(w);
            if let Some((texture, strength)) = current {
                play_texture(id, texture, strength, w);
            }
        }
    }
    for (id, source) in snaps {
        trigger(id, Trigger::Snap, source, w, cx);
    }
}
pub(crate) fn event(
    id: u32,
    event: u32,
    p: &EventPayload,
    proceed: bool,
    w: &mut Window,
    cx: &mut App,
) {
    use tree::*;
    match event {
        EV_MOUSEDOWN if proceed && p.c == 0. => {
            trigger(id, Trigger::Press, input_target(w), w, cx);
            start_texture(id, w);
        }
        EV_CLICK if proceed => {
            let device = if p.key.is_some() {
                HapticTarget::Current
            } else {
                input_target(w)
            };
            trigger(id, Trigger::Activate, device, w, cx);
        }
        EV_MOUSEENTER => {
            trigger(id, Trigger::Hover, input_target(w), w, cx);
        }
        EV_MOUSELEAVE
            if w.vn_pen().is_some()
                && windows::here(w, |s| s.haptics.stroke).is_some_and(|stroke| stroke.0 == id) =>
        {
            end_texture(w)
        }
        EV_MOUSEUP | EV_POINTERCANCEL if w.vn_pen().is_some() => end_texture(w),
        _ => {}
    }
}
