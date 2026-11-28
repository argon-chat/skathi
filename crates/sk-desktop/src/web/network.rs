//! `skathi:network`: `navigator.onLine` plus what the OS knows about the connection, with
//! `change`, `online` and `offline` events. `navigator.connection` is the standard view of the same
//! state: `type` and `change` only, since `effectiveType`, `downlink` and `rtt` would need measuring.

use std::cell::Cell;
use std::ffi::c_void;

use sk_runtime::host;
use sk_runtime::host_fn;
use sk_runtime::redjs::ffi::redjs_protect;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::redjs::webapi::{events, navigator};

use super::{handler_get, handler_set};
use crate::network;

thread_local! {
    static OBJECT: Cell<u64> = const { Cell::new(0) };
    static CONNECTION: Cell<u64> = const { Cell::new(0) };
    /// The `type` `navigator.connection` last announced; unset until a script first asks for the object.
    static CONNECTION_TYPE: Cell<Option<&'static str>> = const { Cell::new(None) };
}

fn tri(value: Option<bool>) -> u64 {
    value.map_or(NULL, enc_bool)
}

host_fn!(fn online(_g, _args) { enc_bool(navigator::on_line()) });
host_fn!(fn internet(g, _args) { unsafe { js_string(g, network::details().internet) } });
host_fn!(fn connection_type(g, _args) { unsafe { js_string(g, network::connection_type()) } });
host_fn!(fn metered(_g, _args) { tri(network::details().metered) });
host_fn!(fn roaming(_g, _args) { tri(network::details().roaming) });
host_fn!(fn over_data_limit(_g, _args) { tri(network::details().over_data_limit) });

fn is_connection(this: u64) -> bool {
    this != 0 && this == CONNECTION.with(Cell::get)
}

host_fn!(fn information_type(g, args) {
    unsafe {
        if !is_connection(args.this().0) {
            return throw_type_error(g, "Illegal invocation");
        }
        js_string(g, network::connection_type())
    }
});

host_fn!(fn get_onchange(g, args) {
    unsafe { handler_get(g, args.this().0, "onchange") }
});

host_fn!(fn set_onchange(g, args) {
    unsafe { handler_set(g, args.this().0, "onchange", args.arg(0).0) };
    UNDEF
});

host_fn!(fn navigator_connection(_g, _args) {
    if CONNECTION_TYPE.with(Cell::get).is_none() {
        CONNECTION_TYPE.with(|t| t.set(Some(network::connection_type())));
    }
    CONNECTION.with(Cell::get)
});

/// `online` and `offline` are also a `change`. `navigator.connection` hears only a change of `type`.
pub(super) unsafe fn on_event(g: *mut c_void, payload: &serde_json::Value) {
    unsafe {
        let object = OBJECT.with(Cell::get);
        let Some(event) = payload["event"].as_str().filter(|_| object != 0) else {
            return;
        };
        // Read before any listener runs: one could change the state, which is the next event's to announce.
        let moved = CONNECTION_TYPE.with(|t| {
            let Some(last) = t.get() else { return false };
            let now = network::connection_type();
            t.set(Some(now));
            now != last
        });
        if event != "change" {
            events::dispatch(g, object, events::new_trusted_event(g, event));
        }
        events::dispatch(g, object, events::new_trusted_event(g, "change"));
        if moved {
            events::dispatch(
                g,
                CONNECTION.with(Cell::get),
                events::new_trusted_event(g, "change"),
            );
        }
    }
}

fn connection_type_named(name: &str) -> Option<&'static str> {
    ["ethernet", "wifi", "cellular", "other", "none", "unknown"]
        .into_iter()
        .find(|&k| k == name)
}

fn internet_named(name: &str) -> Option<&'static str> {
    ["full", "limited", "portal", "none", "unknown"]
        .into_iter()
        .find(|&k| k == name)
}

// `__SK_TEST_NETWORK__({ online?, type?, internet?, metered?, roaming?, overDataLimit? })`, with `SK_NETWORK=fake`.
host_fn!(fn fake(g, args) {
    unsafe {
        let state = args.arg(0).0;
        if !is_cell(state) {
            return throw_type_error(g, "__SK_TEST_NETWORK__ takes an object");
        }
        let flag = |name: &str| match get(g, state, name) {
            UNDEF => Ok(None),
            NULL => Ok(Some(None)),
            TRUE => Ok(Some(Some(true))),
            FALSE => Ok(Some(Some(false))),
            _ => Err(()),
        };
        let text = |name: &str| match get(g, state, name) {
            UNDEF => None,
            v => Some(read_string(g, v).unwrap_or_default()),
        };
        let mut details = network::details();
        let mut touched = false;
        let mut fake = network::Fake::default();
        match get(g, state, "online") {
            UNDEF => {}
            v => fake.online = Some(v == TRUE),
        }
        if let Some(kind) = text("type") {
            let Some(kind) = connection_type_named(&kind) else {
                return throw_type_error(g, "type is not a ConnectionType");
            };
            fake.kind = Some(kind);
        }
        if let Some(internet) = text("internet") {
            let Some(internet) = internet_named(&internet) else {
                return throw_type_error(g, "internet is not an Internet value");
            };
            details.internet = internet;
            touched = true;
        }
        for (name, field) in [
            ("metered", &mut details.metered),
            ("roaming", &mut details.roaming),
            ("overDataLimit", &mut details.over_data_limit),
        ] {
            match flag(name) {
                Ok(None) => {}
                Ok(Some(v)) => {
                    *field = v;
                    touched = true;
                }
                Err(()) => return throw_type_error(g, "the flags take a boolean or null"),
            }
        }
        fake.details = touched.then_some(details);
        network::fake(fake);
        UNDEF
    }
});

pub(super) unsafe fn install(g: *mut c_void, event_target: u64) {
    unsafe {
        let class = define_class(
            g,
            &ClassSpec {
                name: "Network",
                arity: 0,
                construct: None,
                parent_proto: event_target,
                methods: &[],
                getters: &[
                    ("online", online),
                    ("internet", internet),
                    ("type", connection_type),
                    ("metered", metered),
                    ("roaming", roaming),
                    ("overDataLimit", over_data_limit),
                ],
                accessors: &[],
            },
        );
        let object = host::install_object(g, "__SK_NETWORK__", class.proto);
        OBJECT.with(|o| o.set(object));

        let information = define_class(
            g,
            &ClassSpec {
                name: "NetworkInformation",
                arity: 0,
                construct: None,
                parent_proto: event_target,
                methods: &[],
                getters: &[("type", information_type)],
                accessors: &[("onchange", get_onchange, set_onchange)],
            },
        );
        let connection = new_object(g, information.proto);
        redjs_protect(g, connection);
        CONNECTION.with(|c| c.set(connection));
        CONNECTION_TYPE.with(|t| t.set(None));
        super::define_getter(
            g,
            super::navigator_proto(g),
            "connection",
            navigator_connection,
        );

        if network::faked() {
            define(
                g,
                host::global(g),
                "__SK_TEST_NETWORK__",
                make_fn(g, "__SK_TEST_NETWORK__", 1, fake),
                0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use sk_runtime::host_fn;
    use sk_runtime::redjs::webapi::util::*;

    host_fn!(fn fire(g, args) {
        unsafe {
            let event = read_string(g, args.arg(0).0).unwrap_or_default();
            super::on_event(g, &serde_json::json!({ "event": event }));
        }
        UNDEF
    });

    host_fn!(fn set_type(g, args) {
        unsafe {
            let kind = read_string(g, args.arg(0).0).unwrap_or_default();
            crate::network::fake(crate::network::Fake {
                kind: super::connection_type_named(&kind),
                ..Default::default()
            });
        }
        UNDEF
    });

    fn run(src: &str) -> String {
        super::super::testing::run_with(src, |g| unsafe {
            let global = sk_runtime::host::global(g);
            define(
                g,
                global,
                "__sk_test_network_event",
                make_fn(g, "fire", 1, fire),
                0,
            );
            define(
                g,
                global,
                "__sk_test_network_type",
                make_fn(g, "set_type", 1, set_type),
                0,
            );
        })
    }

    #[test]
    fn the_object_reads_the_os_and_an_offline_is_also_a_change() {
        let out = run(
            "const n = __SK_NETWORK__; const seen = []; \
             for (const type of ['online', 'offline', 'change']) n.addEventListener(type, (e) => seen.push(e.type)); \
             __sk_test_network_event('offline'); __sk_test_network_event('change'); \
             return [n instanceof EventTarget, typeof n.online, typeof n.internet, typeof n.type, \
                     [true, false, null].includes(n.metered), seen.join('+')].join('|');",
        );
        assert_eq!(out, "true|boolean|string|string|true|offline+change+change");
    }

    #[test]
    fn navigator_connection_reports_the_provider_type_and_changes_only_with_it() {
        let out = run(
            "__sk_test_network_type('unknown'); const c = navigator.connection, log = []; \
             c.addEventListener('change', () => log.push('listener:' + c.type)); \
             c.onchange = (e) => log.push('on:' + e.isTrusted); \
             __sk_test_network_type('cellular'); __sk_test_network_event('change'); \
             log.push(c.type === __SK_NETWORK__.type); \
             __sk_test_network_event('change'); \
             __sk_test_network_type('none'); __sk_test_network_event('offline'); \
             const absent = ['effectiveType', 'downlink', 'downlinkMax', 'rtt', 'saveData'].filter((k) => k in c); \
             let ctor = false; try { new NetworkInformation(); } catch (e) { ctor = e instanceof TypeError; } \
             let brand = false; \
             try { Object.getOwnPropertyDescriptor(NetworkInformation.prototype, 'type').get.call({}); } \
             catch (e) { brand = e instanceof TypeError; } \
             return [log.join(), absent.length, c === navigator.connection, c instanceof EventTarget, ctor, brand].join('|');",
        );
        assert_eq!(
            out,
            "listener:cellular,on:true,true,listener:none,on:true|0|true|true|true|true"
        );
    }
}
