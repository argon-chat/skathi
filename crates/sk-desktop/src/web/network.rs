//! `skathi:network`: `navigator.onLine` plus what the OS knows about the connection, with
//! `change`, `online` and `offline` events.

use std::cell::Cell;
use std::ffi::c_void;

use sk_runtime::host;
use sk_runtime::host_fn;
use sk_runtime::redjs::webapi::util::*;
use sk_runtime::redjs::webapi::{events, navigator};

use crate::network;

thread_local! {
    static OBJECT: Cell<u64> = const { Cell::new(0) };
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

/// `online` and `offline` are also a `change`.
pub(super) unsafe fn on_event(g: *mut c_void, payload: &serde_json::Value) {
    let object = OBJECT.with(Cell::get);
    let Some(event) = payload["event"].as_str().filter(|_| object != 0) else {
        return;
    };
    if event != "change" {
        events::dispatch(g, object, events::new_trusted_event(g, event));
    }
    events::dispatch(g, object, events::new_trusted_event(g, "change"));
}

pub(super) unsafe fn install(g: *mut c_void, event_target: u64) {
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

    #[test]
    fn the_object_reads_the_os_and_an_offline_is_also_a_change() {
        let out = super::super::testing::run_with(
            "const n = __SK_NETWORK__; const seen = []; \
             for (const type of ['online', 'offline', 'change']) n.addEventListener(type, (e) => seen.push(e.type)); \
             __sk_test_network_event('offline'); __sk_test_network_event('change'); \
             return [n instanceof EventTarget, typeof n.online, typeof n.internet, typeof n.type, \
                     [true, false, null].includes(n.metered), seen.join('+')].join('|');",
            |g| unsafe {
                let global = sk_runtime::host::global(g);
                define(g, global, "__sk_test_network_event", make_fn(g, "fire", 1, fire), 0);
            },
        );
        assert_eq!(out, "true|boolean|string|string|true|offline+change+change");
    }
}
