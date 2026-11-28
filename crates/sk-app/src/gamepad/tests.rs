use super::*;
use sk_gamepad::ButtonChange;

#[test]
#[ignore = "measures a synthetic 1 kHz Steam Controller input stream on the real JS runtime"]
fn benchmark_gamepad_native_replay() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            let ctx = rt.ctx();
            ctx.eval("globalThis.__sk_std ??= {}").unwrap();
            native::install(ctx).unwrap();
            native::clear(ctx);
            ctx.eval(
                r#"
                globalThis.padCount = 0;
                globalThis.padErrors = 0;
                globalThis.padJobs = 0;
                (() => {
                    const tick = Promise.resolve();
                    let dirty = false;
                    __sk_std.gamepads.subscribe(e => {
                        padCount++;
                        if (e.gamepad.axisValues[e.index] !== e.value) padErrors++;
                        if (dirty) return;
                        dirty = true;
                        tick.then(() => { dirty = false; padJobs++ });
                    });
                })();
            "#,
            )
            .unwrap();
            let mut device = pad(1);
            device.buttons.resize(26, 0.0);
            let slots = reconcile(&Rc::default(), [(0, &device)].into_iter(), &[], 0.0);
            let listeners = native::listeners();
            // Sixteen alternating deltas, returning to neutral, in each 16 ms input wave.
            let events = (0..16)
                .map(|i| PadEvent::Axis {
                    instance: 1,
                    index: i % 4,
                    before: if (i / 4) % 2 == 0 { 0.0 } else { 0.75 },
                    recalibrated: false,
                    value: if (i / 4) % 2 == 0 { 0.75 } else { 0.0 },
                })
                .collect();
            let samples = [Sample {
                timestamp: 16.0,
                events,
            }];
            let replay = || native::deliver(ctx, &listeners, &slots, &slots, true, &samples);
            for batched in [false, true] {
                let wave = || if batched { rt.locked(replay) } else { replay() };
                for _ in 0..8 {
                    wave();
                }
                ctx.eval("padCount = padErrors = padJobs = 0").unwrap();
                let mut timings = Vec::with_capacity(128);
                for _ in 0..128 {
                    let start = std::time::Instant::now();
                    wave();
                    timings.push(start.elapsed());
                }
                let elapsed = timings.iter().sum::<Duration>();
                timings.sort_unstable();
                let jobs = ctx.eval("padJobs").unwrap().as_number().unwrap();
                let mode = if batched {
                    "wave lock"
                } else {
                    "per-call locks"
                };
                eprintln!("gamepad replay ({mode}): 2048 axis deltas in {elapsed:?}, {:.0} events/s, 16-event wave p50 {:?}, p95 {:?}, {jobs} microtask flushes", 2048.0 / elapsed.as_secs_f64(), timings[64], timings[121]);
                assert_eq!(ctx.eval("padCount").unwrap().as_number(), Some(2048.0));
                assert_eq!(ctx.eval("padErrors").unwrap().as_number(), Some(0.0));
                if batched {
                    assert_eq!(jobs, 128.0);
                }
            }
            native::clear(ctx);
            SERVICE.with_borrow_mut(|s| *s = Service::default());
            REQUESTED.set(false);
        });
    });
}

fn pad(instance: u32) -> Pad {
    Pad {
        instance,
        name: format!("Pad {instance}"),
        system_id: None,
        dead_zone: 0.0,
        vendor: 0,
        product: 0,
        family: "generic",
        standard: true,
        axes: vec![0.0; 4],
        buttons: vec![0.0; 17],
        can_rumble: false,
    }
}

#[test]
fn dead_zone_js_scopes_validate_and_resolve_connection_aliases() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            let ctx = rt.ctx();
            ctx.eval("globalThis.__sk_std ??= {}").unwrap();
            native::install(ctx).unwrap();
            let mut stable = pad(1);
            stable.system_id = Some("test-endpoint".into());
            let transient = pad(2);
            SERVICE.with_borrow_mut(|s| {
                *s = Service {
                    slots: reconcile(
                        &Rc::default(),
                        [(0, &stable), (1, &transient)].into_iter(),
                        &[],
                        1.,
                    ),
                    focused: true,
                    retry_at: Some(Instant::now() + Duration::from_secs(60)),
                    ..Default::default()
                }
            });
            let ok = ctx.eval(r#"(() => {
                const api = __sk_std.gamepads;
                const global = {scope:"global"};
                const model = {scope:"model", vendorId:0x45e, productId:0xb22};
                const usb = {...model, productId:0xb00};
                const device = {scope:"device", systemId:api.list()[0].systemId};
                const byId = {scope:"device", id:1};
                const transient = {scope:"device", id:2};
                const rejected = fn => { try { fn(); return false } catch { return true } };
                api.setDeadZone(global, 0.1);
                api.setDeadZone(model, 0.2);
                api.setDeadZone(byId, 0);
                api.setDeadZone(transient, 0.3);
                if (api.getDeadZone(device) !== 0 || api.getDeadZone(usb) !== 0.2) return false;
                api.setDeadZone(device, null);
                return api.getDeadZone(byId) === null && api.getDeadZone(transient) === 0.3
                    && api.getDeadZone(global) === 0.1 && api.getDeadZone(model) === 0.2
                    && [NaN, Infinity, 1, -0.1, "0.1", undefined].every(r => rejected(() => api.setDeadZone(global,r)))
                    && [null, {}, {scope:"model",vendorId:0,productId:0}, {...model,productId:65536},
                        {...byId,id:65535}, {...device,id:1}, {...device,systemId:""}]
                       .every(s => rejected(() => api.getDeadZone(s)))
                    && api.getDeadZone(global) === 0.1;
            })()"#).unwrap();
            assert_eq!(ok.as_bool(), Some(true));
            SERVICE.with_borrow_mut(|s| s.focused = false);
            assert!(ctx
                .eval("__sk_std.gamepads.setDeadZone({scope:'device',id:2}, 0.1)")
                .is_err());
            assert_eq!(
                ctx.eval("__sk_std.gamepads.getDeadZone({scope:'device',systemId:'test-endpoint'}) === null")
                    .unwrap().as_bool(), Some(true)
            );
            SERVICE.with_borrow_mut(|s| *s = Service::default());
            REQUESTED.set(false);
        });
    });
}

#[test]
fn gamepad_slots_survive_reordering_and_reuse_disconnected_indices() {
    let devices = [pad(10), pad(20), pad(30)];
    let indexed = [(0, &devices[0]), (1, &devices[1]), (2, &devices[2])];
    let slots = reconcile(&Rc::default(), indexed.into_iter(), &[], 2.0);
    assert!(Rc::ptr_eq(
        &slots,
        &reconcile(&slots, indexed.into_iter().rev(), &[], 3.0)
    ));
    let mut changed = devices[2].clone();
    changed.axes[1] = -0.75;
    let new = pad(40);
    let next = [(0, &new), (2, &changed), (1, &devices[1])];
    let slots = reconcile(&slots, next.into_iter(), &[], 4.0);
    let observed: Vec<_> = slots
        .iter()
        .map(|s| (s.index, s.pad.instance, s.timestamp))
        .collect();
    assert_eq!(observed, [(0, 40, 4.0), (1, 20, 2.0), (2, 30, 4.0)]);
    let tap = [
        ButtonChange {
            instance: 20,
            index: 0,
            before: 0.,
            value: 1.,
        },
        ButtonChange {
            instance: 20,
            index: 0,
            before: 1.,
            value: 0.,
        },
    ];
    let tapped = reconcile(&slots, next.into_iter(), &tap.map(PadEvent::Button), 5.0);
    assert_eq!(tapped[1].timestamp, 5.0);
    assert_eq!(tapped[2].timestamp, 4.0);
}

#[test]
fn gamepad_native_subscription_skips_buttons_queued_before_registration() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            let ctx = rt.ctx();
            ctx.eval("globalThis.__sk_std ??= {}").unwrap();
            native::install(ctx).unwrap();
            native::clear(ctx);
            web::install(rt);
            web::shutdown(rt);
            let tap = [
                ButtonChange {
                    instance: 17,
                    index: 0,
                    before: 0.,
                    value: 1.,
                },
                ButtonChange {
                    instance: 17,
                    index: 0,
                    before: 1.,
                    value: 0.,
                },
            ];
            let slots = reconcile(&Rc::default(), [(0, &pad(17))].into_iter(), &[], 1.);
            let sample = || Sample {
                timestamp: 2.,
                events: tap.map(PadEvent::Button).into(),
            };
            SERVICE.with_borrow_mut(|s| {
                *s = Service {
                    slots: slots.clone(),
                    delivered: slots,
                    focused: true,
                    retry_at: Some(Instant::now() + Duration::from_secs(10)),
                    samples: vec![sample()],
                    ..Default::default()
                }
            });
            ctx.eval(
                r#"globalThis.padEdges = [];
                   globalThis.firstCheckpointEdges = "";
                   __sk_std.gamepads.subscribe(e => {
                       padEdges.push(e.edge);
                       if (e.edge === "down") Promise.resolve().then(() => {
                           firstCheckpointEdges = padEdges.join(",");
                       });
                   })"#,
            )
            .unwrap();
            SERVICE.with_borrow_mut(|s| s.samples.push(sample()));
            // A focused pop-out still receives native input when the main document is hidden.
            pump(rt, true, false);
            assert_eq!(
                ctx.eval("firstCheckpointEdges === 'down,up'")
                    .unwrap()
                    .as_bool(),
                Some(true)
            );
            assert_eq!(
                ctx.eval("navigator.getGamepads().length === 0")
                    .unwrap()
                    .as_bool(),
                Some(true)
            );
            assert_eq!(
                ctx.eval("padEdges.join() === 'down,up'").unwrap().as_bool(),
                Some(true)
            );
            clear(ctx, false);
        })
    });
}

#[test]
fn gamepad_native_stream_preserves_interleaved_taps_and_disconnects() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            let ctx = rt.ctx();
            ctx.eval("globalThis.__sk_std ??= {}").unwrap();
            native::install(ctx).unwrap();
            native::clear(ctx);
            ctx.eval("globalThis.padLog = []; globalThis.padOff = __sk_std.gamepads.subscribe(e => padLog.push([e.type, e.gamepad.id, e.edge ?? '', e.gamepad.buttons.south].join('/')))").unwrap();
            let devices = [pad(10), pad(20)];
            let slots = reconcile(
                &Rc::default(),
                [(0, &devices[0]), (1, &devices[1])].into_iter(),
                &[],
                1.0,
            );
            let mut held = slots[0].clone();
            held.pad.buttons[0] = 1.0;
            let listeners = native::listeners();
            native::deliver(ctx, &listeners, &[], &[held], true, &[]);
            let edge = |instance, before, value| ButtonChange {
                instance,
                index: 0,
                before,
                value,
            };
            let taps = [
                edge(20, 0., 1.),
                edge(10, 0., 1.),
                edge(20, 1., 0.),
                edge(10, 1., 0.),
            ];
            native::deliver(
                ctx,
                &listeners,
                &slots,
                &slots,
                true,
                &[Sample {
                    timestamp: 2.,
                    events: taps.map(PadEvent::Button).into(),
                }],
            );
            native::deliver(ctx, &listeners, &slots, &slots[1..], true, &[]);
            assert_eq!(ctx.eval("padLog.join() === 'connected/10//1,button/20/down/1,button/10/down/1,button/20/up/0,button/10/up/0,disconnected/10//0'").unwrap().as_bool(), Some(true));
            ctx.eval("padOff(); padOff()").unwrap();
            native::deliver(ctx, &listeners, &slots, &[], true, &[]);
            assert_eq!(
                ctx.eval("padLog.length === 6 && __sk_std.gamepads.rumble(42) === false")
                    .unwrap()
                    .as_bool(),
                Some(true)
            );
            assert!(ctx.eval("__sk_std.gamepads.rumble(42, Symbol())").is_err());
            assert!(!native::observed());
            SERVICE.with_borrow_mut(|s| *s = Service::default());
            REQUESTED.set(false);
        })
    });
}

#[test]
fn gamepad_native_delivers_input_and_short_lived_connections_before_disconnect() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            let ctx = rt.ctx();
            ctx.eval("globalThis.__sk_std ??= {}").unwrap();
            native::install(ctx).unwrap();
            native::clear(ctx);
            ctx.eval(r#"
            globalThis.padHistory = [];
            globalThis.calibratedRadius = null;
            __sk_std.gamepads.subscribe(({type, gamepad, edge}) => {
                padHistory.push([type, gamepad.index, gamepad.id, edge ?? "", gamepad.buttons.south, gamepad.axisValues[0]].join("/"));
                if (type === "axis") calibratedRadius = gamepad.deadZone;
            });
        "#).unwrap();
            let old = reconcile(&Rc::default(), [(0, &pad(10))].into_iter(), &[], 1.);
            let button = |instance, before, value| {
                PadEvent::Button(ButtonChange {
                    instance,
                    index: 0,
                    before,
                    value,
                })
            };
            let mut transient = pad(20);
            transient.axes[0] = 0.5;
            transient.buttons[0] = 1.;
            transient.dead_zone = 0.1;
            let events = vec![
                button(10, 0., 1.),
                PadEvent::Connected {
                    index: 1,
                    pad: pad(20),
                },
                PadEvent::DeadZone {
                    instance: 20,
                    radius: 0.1,
                },
                PadEvent::Axis {
                    instance: 20,
                    index: 0,
                    before: 0.0,
                    recalibrated: false,
                    value: 0.5,
                },
                button(20, 0., 1.),
                button(10, 1., 0.),
                PadEvent::Disconnected(pad(10)),
                PadEvent::Connected {
                    index: 0,
                    pad: pad(30),
                },
                PadEvent::Disconnected(transient),
            ];
            let current = reconcile(&old, [(0, &pad(30))].into_iter(), &events, 2.);
            let samples = [Sample {
                timestamp: 2.,
                events,
            }];
            let listeners = native::listeners();
            native::deliver(ctx, &listeners, &old, &current, true, &samples);
            assert_eq!(ctx.eval("calibratedRadius").unwrap().as_number(), Some(0.1));
            let expected = [
                "button/0/10/down/1/0",
                "connected/1/20//0/0",
                "axis/1/20//0/0.5",
                "button/1/20/down/1/0.5",
                "button/0/10/up/0/0",
                "disconnected/0/10//0/0",
                "connected/0/30//0/0",
                "disconnected/1/20//1/0.5",
            ];
            assert_eq!(
                ctx.read_string(ctx.eval("padHistory.join()").unwrap())
                    .unwrap(),
                expected.join(",")
            );
            ctx.eval("padHistory.length = 0").unwrap();
            native::deliver(ctx, &listeners, &old, &current, false, &samples);
            assert_eq!(
                ctx.read_string(ctx.eval("padHistory.join()").unwrap())
                    .unwrap(),
                "disconnected/0/10//0/0,connected/0/30//0/0"
            );
            clear(ctx, false);
        })
    });
}

#[test]
fn gamepad_replay_is_stable_when_a_listener_reads_or_changes_subscriptions() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            let ctx = rt.ctx();
            ctx.eval("globalThis.__sk_std ??= {}").unwrap();
            native::install(ctx).unwrap();
            native::clear(ctx);
            let slots = reconcile(&Rc::default(), [(0, &pad(10))].into_iter(), &[], 1.0);
            SERVICE.with_borrow_mut(|s| {
                *s = Service {
                    slots: slots.clone(),
                    ..Default::default()
                }
            });
            ctx.eval(
                r#"
                globalThis.replay = { events: [], polls: [], removed: 0, added: 0 };
                (() => {
                    const host = __sk_std.gamepads;
                    host.subscribe(e => {
                        if (!replay.events.length) {
                            remove();
                            host.subscribe(() => replay.added++);
                        }
                        replay.events.push(e);
                        replay.polls.push(host.list()[0].buttons.south);
                    });
                    const remove = host.subscribe(() => replay.removed++);
                })();
            "#,
            )
            .unwrap();
            let samples = [Sample {
                timestamp: 2.0,
                events: [(0.0, 1.0), (1.0, 0.0)]
                    .map(|(before, value)| {
                        PadEvent::Button(ButtonChange {
                            instance: 10,
                            index: 0,
                            before,
                            value,
                        })
                    })
                    .into(),
            }];
            let listeners = native::listeners();
            rt.locked(|| native::deliver(ctx, &listeners, &slots, &slots, true, &samples));
            assert_eq!(
                ctx.eval(
                    r#"
                replay.removed === 0 && replay.added === 0 && replay.polls.join() === '0,0' &&
                replay.events.map(e => e.gamepad.buttons.south).join() === '1,0' &&
                replay.events.map(e => e.edge).join() === 'down,up' &&
                replay.events[0].gamepad !== replay.events[1].gamepad
            "#
                )
                .unwrap()
                .as_bool(),
                Some(true)
            );
            let listeners = native::listeners();
            rt.locked(|| native::deliver(ctx, &listeners, &slots, &[], true, &[]));
            assert_eq!(ctx.eval("replay.added").unwrap().as_number(), Some(1.0));
            native::clear(ctx);
            SERVICE.with_borrow_mut(|s| *s = Service::default());
            REQUESTED.set(false);
        });
    });
}
