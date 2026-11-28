use super::*;
use sk_runtime::redjs::{capi::JSClassRef, glue};
use std::sync::OnceLock;

#[derive(Copy, Clone, Default)]
pub(super) struct Realm {
    context: u64,
    pub(super) navigator: u64,
    pad: u64,
    button: u64,
    event: u64,
    actuator: u64,
    freeze: u64,
    empty: u64,
    supported: u64,
    axes_key: usize,
    buttons_key: usize,
    actuator_key: usize,
    event_pad: usize,
    connected_handler: usize,
    disconnected_handler: usize,
}

thread_local! { static REALM: Cell<Realm> = Cell::default(); }
pub(super) fn realm() -> Realm {
    REALM.get()
}

struct PadView {
    slot: Arc<Slot>,
    connected: bool,
    session: u64,
}
#[derive(Copy, Clone)]
pub(super) struct Actuator {
    pub instance: u32,
    pub supported: bool,
    session: u64,
}
impl Actuator {
    pub(super) fn active(self) -> bool {
        self.session == SESSION.get() && usable(self.instance)
    }
}

unsafe fn pad_class() -> JSClassRef {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe { make_host_class::<PadView>("Gamepad") as usize }) as JSClassRef
}
unsafe fn button_class() -> JSClassRef {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe { make_host_class::<f64>("GamepadButton") as usize }) as JSClassRef
}
unsafe fn actuator_class() -> JSClassRef {
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe { make_host_class::<Actuator>("GamepadHapticActuator") as usize })
        as JSClassRef
}

pub(super) unsafe fn pad(g: *mut c_void, slot: Arc<Slot>, connected: bool) -> u64 {
    unsafe {
        host_create(
            g,
            pad_class(),
            realm().pad,
            PadView {
                slot,
                connected,
                session: SESSION.get(),
            },
        )
    }
}
pub(super) unsafe fn set_connected(g: *mut c_void, object: u64, value: bool) {
    unsafe {
        host_data::<PadView>(g, pad_class(), object)
            .unwrap()
            .connected = value
    };
}
pub(super) unsafe fn refresh(g: *mut c_void, object: u64, slot: Arc<Slot>) {
    unsafe {
        let data = host_data::<PadView>(g, pad_class(), object).unwrap();
        let axes_changed = data.slot.pad.axes != slot.pad.axes;
        let buttons_changed = data.slot.pad.buttons != slot.pad.buttons;
        data.slot = slot;
        if axes_changed {
            glue::rjs_define(g, object, realm().axes_key as _, UNDEF, 0);
        }
        if buttons_changed {
            glue::rjs_define(g, object, realm().buttons_key as _, UNDEF, 0);
        }
    }
}

unsafe fn lazy(g: *mut c_void, this: u64, key: usize, make: impl FnOnce() -> u64) -> u64 {
    unsafe {
        let cached = get_a(g, this, key as _);
        if cached != UNDEF {
            return cached;
        }
        let value = make();
        glue::rjs_define(g, this, key as _, value, 0);
        value
    }
}

unsafe fn frozen_array(g: *mut c_void, freeze: u64, items: impl Iterator<Item = u64>) -> u64 {
    unsafe {
        let array = new_array(g);
        redjs_protect(g, array);
        for (i, v) in items.enumerate() {
            set_i(g, array, i as u32, v);
        }
        let _ = call(g, freeze, UNDEF, &[array]);
        redjs_unprotect(g, array);
        array
    }
}

macro_rules! pad_getter {
    ($name:ident, |$g:ident, $this:ident, $pad:ident| $body:expr) => {
        host_fn!(fn $name($g, args) {
            unsafe {
                let $this = args.this().0;
                let Some($pad) = host_data::<PadView>($g, pad_class(), $this) else {
                    return throw_type_error($g, "Illegal invocation");
                };
                $body
            }
        });
    };
}
pad_getter!(id, |g, this, p| js_string(g, &p.slot.pad.name));
pad_getter!(index, |g, this, p| enc_f64(f64::from(p.slot.index)));
pad_getter!(timestamp, |g, this, p| enc_f64(p.slot.timestamp));
pad_getter!(connected, |g, this, p| enc_bool(p.connected));
pad_getter!(mapping, |g, this, p| js_string(
    g,
    if p.slot.pad.standard { "standard" } else { "" }
));
pad_getter!(axes, |g, this, p| {
    let r = realm();
    let slot = p.slot.clone();
    lazy(g, this, r.axes_key, || {
        frozen_array(g, r.freeze, slot.pad.axes.iter().copied().map(enc_f64))
    })
});
pad_getter!(buttons, |g, this, p| {
    let r = realm();
    let slot = p.slot.clone();
    lazy(g, this, r.buttons_key, || {
        frozen_array(
            g,
            r.freeze,
            slot.pad
                .buttons
                .iter()
                .map(|&v| host_create(g, button_class(), r.button, v)),
        )
    })
});
pad_getter!(touches, |g, this, _p| realm().empty);
pad_getter!(vibration_actuator, |g, this, p| {
    let r = realm();
    let actuator = Actuator {
        instance: p.slot.pad.instance,
        supported: p.slot.pad.can_rumble,
        session: p.session,
    };
    lazy(g, this, r.actuator_key, || {
        host_create(g, actuator_class(), r.actuator, actuator)
    })
});

macro_rules! button_getter {
    ($name:ident, |$v:ident| $out:expr) => {
        host_fn!(fn $name(g, args) {
            unsafe {
                let Some($v) = host_data::<f64>(g, button_class(), args.this().0).copied() else { return throw_type_error(g, "Illegal invocation") };
                $out
            }
        });
    };
}
button_getter!(value, |v| enc_f64(v));
button_getter!(pressed, |v| enc_bool(v > 0.5));

pub(super) unsafe fn actuator(g: *mut c_void, this: u64) -> Option<Actuator> {
    unsafe { host_data::<Actuator>(g, actuator_class(), this).copied() }
}
host_fn!(fn effects(g, args) {
    unsafe {
        let Some(actuator) = actuator(g, args.this().0) else { return throw_type_error(g, "Illegal invocation") };
        if actuator.supported { realm().supported } else { realm().empty }
    }
});
host_fn!(fn actuator_type(g, args) {
    unsafe {
        if actuator(g, args.this().0).is_none() { return throw_type_error(g, "Illegal invocation") }
        js_string(g, "dual-rumble")
    }
});

host_fn!(fn event_construct(g, args) {
    JsRuntime::with(|rt| unsafe {
        if args.argc() == 0 { return throw_type_error(g, "GamepadEvent requires a type") }
        let mut roots = crate::tray::Roots::new(rt.ctx());
        let Ok(ty) = to_js_string(g, args.arg(0).0) else { return UNDEF };
        roots.keep(crate::js::Value(ty));
        let init = args.arg(1).0;
        let gamepad = if matches!(init, UNDEF | NULL) {
            NULL
        } else {
            if glue::rjs_kind(g, init) < 8 { return throw_type_error(g, "Expected GamepadEventInit") }
            match try_get(g, init, "gamepad") {
                Ok(UNDEF | NULL) => NULL,
                Ok(pad) if host_data::<PadView>(g, pad_class(), pad).is_some() => pad,
                Ok(_) => return throw_type_error(g, "Expected a Gamepad"),
                Err(_) => return UNDEF,
            }
        };
        roots.keep(crate::js::Value(gamepad));
        let r = realm();
        let object = ordinary_create(g, args.this().0, r.event);
        roots.keep(crate::js::Value(object));
        events::init_event(g, object, ty, init, false);
        glue::rjs_define(g, object, r.event_pad as _, gamepad, 0);
        object
    })
});
host_fn!(fn event_gamepad(g, args) {
    unsafe {
        let mut result = UNDEF;
        if glue::rjs_get_own_data(g, args.this().0, realm().event_pad as _, &mut result) == 1 {
            result
        } else {
            throw_type_error(g, "Illegal invocation")
        }
    }
});
pub(super) unsafe fn dispatch(g: *mut c_void, name: &str, pad: u64) {
    unsafe {
        let r = realm();
        let event = new_object(g, r.event);
        redjs_protect(g, event);
        events::init_event(g, event, js_string(g, name), UNDEF, false);
        if let Some(s) = events::state_of(g, event) {
            define(g, s, "isTrusted", TRUE, 0);
        }
        glue::rjs_define(g, event, r.event_pad as _, pad, 0);
        events::dispatch(g, host::global(g), event);
        redjs_unprotect(g, event);
    }
}

macro_rules! handler {
    ($getter:ident, $setter:ident, $field:ident, $event:literal) => {
        host_fn!(fn $getter(g, _args) { unsafe {
            let value = get_a(g, host::global(g), realm().$field as _);
            if value == UNDEF { NULL } else { value }
        }});
        host_fn!(fn $setter(g, args) { unsafe {
            let value = args.arg(0).0;
            let cb = if events::is_callable(g, value) { observe_listener($event); value } else { NULL };
            glue::rjs_define(g, host::global(g), realm().$field as _, cb, 0);
            UNDEF
        }});
    };
}
handler!(
    on_connect,
    set_on_connect,
    connected_handler,
    "gamepadconnected"
);
handler!(
    on_disconnect,
    set_on_disconnect,
    disconnected_handler,
    "gamepaddisconnected"
);

pub(super) fn clear_handlers(g: *mut c_void) {
    unsafe {
        for key in [realm().connected_handler, realm().disconnected_handler] {
            if key != 0 {
                glue::rjs_define(g, host::global(g), key as _, NULL, 0);
            }
        }
    }
}

pub(super) fn install(rt: &JsRuntime) {
    let g = rt.raw();
    if realm().context == rt.ctx().id().0 && realm().navigator != 0 {
        return;
    }
    // SAFETY: all per-realm prototypes and intrinsics are rooted for the lifetime of the JSC context.
    unsafe {
        let global = host::global(g);
        let proto = |name, methods: &[(&str, u32, HostFn)], getters: &[(&str, HostFn)]| {
            define_class(
                g,
                &ClassSpec {
                    name,
                    arity: 0,
                    parent_proto: 0,
                    construct: None,
                    methods,
                    getters,
                    accessors: &[],
                },
            )
            .proto
        };
        let pad = proto(
            "Gamepad",
            &[],
            &[
                ("id", id),
                ("index", index),
                ("connected", connected),
                ("timestamp", timestamp),
                ("mapping", mapping),
                ("axes", axes),
                ("buttons", buttons),
                ("touches", touches),
                ("vibrationActuator", vibration_actuator),
            ],
        );
        let button = proto(
            "GamepadButton",
            &[],
            &[("value", value), ("pressed", pressed), ("touched", pressed)],
        );
        let actuator = proto(
            "GamepadHapticActuator",
            &[
                ("playEffect", 1, haptics::play_effect),
                ("reset", 0, haptics::reset),
            ],
            &[("effects", effects), ("type", actuator_type)],
        );
        let event = define_class(
            g,
            &ClassSpec {
                name: "GamepadEvent",
                arity: 1,
                parent_proto: state(g).event.proto,
                construct: Some(event_construct),
                methods: &[],
                getters: &[("gamepad", event_gamepad)],
                accessors: &[],
            },
        )
        .proto;
        let navigator = get(g, global, "navigator");
        let object = get(g, global, "Object");
        let freeze = get(g, object, "freeze");
        let empty = frozen_array(g, freeze, std::iter::empty());
        redjs_protect(g, empty);
        let supported = frozen_array(g, freeze, std::iter::once(js_string(g, "dual-rumble")));
        for value in [navigator, pad, button, actuator, event, freeze, supported] {
            redjs_protect(g, value);
        }
        let key = |name| atom_priv(g, name) as usize;
        REALM.set(Realm {
            context: rt.ctx().id().0,
            navigator,
            pad,
            button,
            actuator,
            event,
            freeze,
            supported,
            empty,
            axes_key: key("Gamepad axes"),
            buttons_key: key("Gamepad buttons"),
            actuator_key: key("Gamepad actuator"),
            event_pad: key("GamepadEvent pad"),
            connected_handler: key("Window ongamepadconnected"),
            disconnected_handler: key("Window ongamepaddisconnected"),
        });
        let navigator_proto = get(g, get(g, global, "Navigator"), "prototype");
        define(
            g,
            navigator_proto,
            "getGamepads",
            make_fn(g, "getGamepads", 0, get_gamepads),
            2,
        );
        for (name, read, write) in [
            (
                "ongamepadconnected",
                on_connect as HostFn,
                set_on_connect as HostFn,
            ),
            ("ongamepaddisconnected", on_disconnect, set_on_disconnect),
        ] {
            let key = atom(g, name);
            glue::rjs_define_accessor(
                g,
                global,
                key,
                make_fn(g, &format!("get {name}"), 0, read),
                make_fn(g, &format!("set {name}"), 1, write),
                0,
            );
            glue::rjs_atom_free(g, key);
        }
    }
}
