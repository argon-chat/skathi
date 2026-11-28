use super::*;
use sk_gamepad::{ButtonChange, Pad};

fn pad(instance: u32, index: u32, value: f64, timestamp: f64) -> Slot {
    Slot {
        index,
        timestamp,
        pad: Pad {
            instance,
            name: format!("Test controller {instance}"),
            system_id: None,
            dead_zone: 0.0,
            vendor: 0,
            product: 0,
            family: "generic",
            standard: true,
            axes: vec![0.; 4],
            buttons: vec![value, 0.],
            can_rumble: true,
        },
    }
}

fn check(rt: &JsRuntime, expression: &str) {
    assert_eq!(
        rt.ctx().eval(expression).unwrap().as_bool(),
        Some(true),
        "{expression}"
    );
}

#[test]
fn recalibrating_a_held_stick_does_not_unlock_web_gamepads() {
    let mut before = pad(5, 0, 0., 1.);
    before.pad.dead_zone = 0.5;
    before.pad.axes[0] = 0.25;
    let mut after = before.clone();
    after.pad.dead_zone = 0.;
    after.pad.axes[0] = 0.625;
    assert!(gesture(&before, &after, &[]).is_none());
    after.pad.buttons[0] = 1.;
    assert!(gesture(&before, &after, &[]).is_some());
}

#[test]
fn gamepad_web_only_physical_stick_crossings_unlock_input() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            install(rt);
            let axis = |before, value, recalibrated| PadEvent::Axis {
                instance: 5,
                index: 0,
                before,
                value,
                recalibrated,
            };
            for calibration_first in [false, true] {
                shutdown(rt);
                let mut slot = pad(5, 0, 0., 1.);
                slot.pad.axes[0] = 0.4;
                let events = if calibration_first {
                    vec![
                        axis(0.4, 0.5, true),
                        axis(0.5, -0.7, false),
                        axis(-0.7, 0.4, false),
                    ]
                } else {
                    vec![axis(0.4, -0.7, false), axis(-0.7, 0.4, true)]
                };
                let movement = [
                    Sample {
                        timestamp: 4.,
                        events,
                    },
                    Sample {
                        timestamp: 5.,
                        events: vec![axis(0.4, 0.3, false)],
                    },
                ];
                update(rt, std::slice::from_ref(&slot), true, &movement);
                update(rt, std::slice::from_ref(&slot), false, &movement);
                update(rt, std::slice::from_ref(&slot), true, &movement);
                check(rt, "navigator.getGamepads().length === 0");

                // A threshold crossing solely from calibration leaves the device private, even
                // when the radius changed twice and ended at the old value in this wave.
                slot.pad.axes[0] = 0.6;
                slot.timestamp = 2.;
                let synthetic = [Sample {
                    timestamp: 2.,
                    events: vec![axis(0.4, 0.6, true)],
                }];
                update(rt, std::slice::from_ref(&slot), true, &synthetic);
                check(rt, "navigator.getGamepads().length === 0");
                slot.pad.axes[0] = 0.4;
                slot.timestamp = 3.;
                update(rt, std::slice::from_ref(&slot), true, &[]);

                slot.pad.dead_zone = 0.3;
                slot.pad.axes[0] = 0.3;
                slot.timestamp = 5.;
                update(rt, std::slice::from_ref(&slot), true, &movement);
                check(rt, "navigator.getGamepads()[0].axes[0] === 0.3 && navigator.getGamepads()[0].timestamp === 5");
            }
            shutdown(rt);
        });
    });
}

#[test]
fn gamepad_web_events_snapshots_and_reconnects() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            install(rt);
            shutdown(rt);
            let cx = rt.ctx();
            cx.eval("globalThis.padEvents = []; ongamepadconnected = e => padEvents.push(e); ongamepaddisconnected = e => { padEvents.push(e); globalThis.disconnectSnapshot = navigator.getGamepads()[e.gamepad.index]; }").unwrap();
            assert!(observed(rt));
            update(rt, &[pad(11, 0, 1., 1.)], true, &[]);
            check(
                rt,
                "navigator.getGamepads().length === 0 && padEvents.length === 0",
            );
            update(rt, &[pad(11, 0, 0., 2.), pad(12, 1, 0., 2.)], true, &[]);
            update(rt, &[pad(11, 0, 1., 3.), pad(12, 1, 0., 2.)], true, &[]);
            check(rt, "padEvents.length === 2 && padEvents[0].isTrusted && padEvents[0] instanceof Event && padEvents[0] instanceof GamepadEvent");
            cx.eval("globalThis.padOld = navigator.getGamepads()[0]; globalThis.padLive = padEvents[0].gamepad").unwrap();
            check(rt, "padOld instanceof Gamepad && padOld.id === 'Test controller 11' && padOld.index === 0 && padOld.connected && padOld.timestamp === 3 && padOld.mapping === 'standard'");
            check(rt, "padOld.buttons[0] instanceof GamepadButton && padOld.buttons[0].pressed && padOld.buttons[0].touched && padOld.buttons[0].value === 1");
            check(rt, "Object.isFrozen(padOld.axes) && Object.isFrozen(padOld.buttons) && padOld.buttons === padOld.buttons && padOld.axes === padOld.axes");
            check(rt, "new GamepadEvent('x', {gamepad:padOld,bubbles:true}).gamepad === padOld && new GamepadEvent('x').gamepad === null");
            assert!(cx.eval("new GamepadEvent('x', {gamepad:{}})").is_err());
            assert!(cx.eval("new Gamepad()").is_err());
            update(rt, &[pad(11, 0, 0., 4.), pad(12, 1, 0., 2.)], true, &[]);
            check(rt, "padOld.buttons[0].pressed && !padLive.buttons[0].pressed && !navigator.getGamepads()[0].buttons[0].pressed && padLive.timestamp === 4");
            update(rt, &[pad(12, 1, 0., 2.)], true, &[]);
            check(rt, "padEvents[2].type === 'gamepaddisconnected' && padEvents[2].gamepad === padLive && !padLive.connected && navigator.getGamepads()[0] === null && navigator.getGamepads()[1].index === 1");
            check(
                rt,
                "disconnectSnapshot.id === padLive.id && !disconnectSnapshot.connected",
            );
            update(rt, &[pad(21, 0, 0., 5.), pad(12, 1, 0., 2.)], true, &[]);
            check(
                rt,
                "padEvents[3].gamepad.id === 'Test controller 21' && !padLive.connected",
            );
            update(rt, &[pad(21, 0, 0., 5.)], true, &[]);
            check(rt, "navigator.getGamepads().length === 1");
            update(rt, &[], true, &[]);
            check(rt, "navigator.getGamepads().length === 0");
            shutdown(rt);
            check(rt, "ongamepadconnected === null && ongamepaddisconnected === null && navigator.getGamepads().length === 0");
        })
    });
}

#[test]
fn gamepad_web_quick_tap_between_samples() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            install(rt);
            shutdown(rt);
            let mut slots = [pad(13, 0, 0., 1.)];
            slots[0].pad.standard = false;
            slots[0].pad.buttons.clear();
            let press = ButtonChange {
                instance: 13,
                index: 0,
                before: 0.,
                value: 1.,
            };
            let tap = [Sample {
                timestamp: 2.,
                events: vec![
                    PadEvent::Button(press),
                    PadEvent::Button(ButtonChange {
                        before: 1.,
                        value: 0.,
                        ..press
                    }),
                ],
            }];
            update(rt, &slots, true, &tap);
            update(rt, &slots, false, &tap);
            update(rt, &slots, true, &tap);
            check(rt, "navigator.getGamepads().length === 0");

            // Unmapped button indices become known on first input, including a complete tap.
            slots[0].pad.buttons.push(0.);
            rt.ctx().eval("globalThis.quickTaps = 0; addEventListener('gamepadconnected', e => { ++quickTaps; globalThis.quickPad = e.gamepad; }, {once:true})").unwrap();
            assert!(observed(rt));
            update(rt, &slots, true, &tap);
            check(
                rt,
                "quickTaps === 1 && navigator.getGamepads()[0].mapping === '' && !navigator.getGamepads()[0].buttons[0].pressed",
            );
            assert!(!observed(rt));
            rt.ctx().eval("ongamepaddisconnected = () => {}").unwrap();
            assert!(observed(rt));
            rt.ctx().eval("ongamepaddisconnected = null").unwrap();
            assert!(!observed(rt));
            rt.ctx()
                .eval("globalThis.firstTap = navigator.getGamepads()[0]")
                .unwrap();
            slots[0].timestamp = PAGE.with_borrow(|p| p.devices[0].state.timestamp + 1.);
            update(rt, &slots, true, &tap);
            check(rt, "quickPad.timestamp > firstTap.timestamp && quickPad.timestamp === navigator.getGamepads()[0].timestamp");
            shutdown(rt);
        })
    });
}

#[test]
fn gamepad_web_haptics_preemption_and_focus() {
    sk_runtime::testing::on_js_thread(|| {
        JsRuntime::with(|rt| {
            install(rt);
            shutdown(rt);
            update(rt, &[pad(1, 0, 0., 1.)], true, &[]);
            update(rt, &[pad(1, 0, 1., 2.)], false, &[]);
            update(rt, &[pad(1, 0, 1., 2.)], true, &[]);
            check(rt, "navigator.getGamepads().length === 0");
            update(rt, &[pad(1, 0, 0., 3.)], true, &[]);
            update(rt, &[pad(1, 0, 1., 4.)], true, &[]);
            let cx = rt.ctx();
            cx.eval("globalThis.padResults=[]; globalThis.padAct = navigator.getGamepads()[0].vibrationActuator; padAct.playEffect('dual-rumble',{duration:100,startDelay:1000}).then(v=>padResults.push(v)); padAct.playEffect('dual-rumble',{strongMagnitude:2}).catch(e=>padResults.push(e.name))").unwrap();
            cx.drain_microtasks();
            check(rt, "padAct instanceof GamepadHapticActuator && padAct.effects[0] === 'dual-rumble' && Object.isFrozen(padAct.effects) && padResults.join() === 'TypeError'");
            assert!(pending());
            update(rt, &[pad(1, 0, 1., 4.)], false, &[]);
            cx.drain_microtasks();
            check(rt, "padResults.join() === 'TypeError,preempted'");
            assert!(!pending());
            cx.eval("padAct.reset().catch(e=>padResults.push(e.name))")
                .unwrap();
            cx.drain_microtasks();
            check(rt, "padResults.at(-1) === 'InvalidStateError'");
            update(rt, &[pad(1, 0, 1., 4.)], true, &[]);
            cx.eval("padAct.playEffect('trigger-rumble').catch(e=>padResults.push(e.name))")
                .unwrap();
            cx.drain_microtasks();
            check(rt, "padResults.at(-1) === 'NotSupportedError'");
            shutdown(rt);
        })
    });
}
