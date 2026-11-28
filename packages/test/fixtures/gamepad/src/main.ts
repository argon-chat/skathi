import { createApp, h } from "@skathi-native/runtime";
import { gamepads, type DeadZoneTarget } from "skathi:gamepad";
import { Runtime } from "skathi:runtime";

function deadZoneOverrides() {
  const target: DeadZoneTarget = { scope: "global" };
  const previous = gamepads.getDeadZone(target);
  try {
    gamepads.setDeadZone(target, 0);
    const disabled = gamepads.getDeadZone(target) === 0;
    gamepads.setDeadZone(target, null);
    return disabled && gamepads.getDeadZone(target) === null;
  } finally {
    gamepads.setDeadZone(target, previous);
  }
}

function probe() {
  const initial = navigator.getGamepads?.();
  const connected = gamepads.list();
  const unsubscribe = gamepads.subscribe(() => {});
  unsubscribe();
  unsubscribe();

  const event = new GamepadEvent("gamepadconnected");
  let delivered = 0;
  window.addEventListener(
    "gamepadconnected",
    (e) => {
      if (e === event) delivered++;
    },
    { once: true },
  );
  window.dispatchEvent(event);
  window.dispatchEvent(event);
  return {
    jit: !Runtime.isAot(),
    native: gamepads.available && Array.isArray(connected),
    deadZones: deadZoneOverrides(),
    web: Array.isArray(initial) && initial.length === 0,
    event: event instanceof Event && !event.isTrusted && event.gamepad === null && delivered === 1,
    staleOutput: gamepads.rumble(0xffffffff, { durationMs: 0 }),
  };
}

Object.assign(globalThis, { probeGamepads: probe });
createApp({ render: () => h("div", { id: "ready" }, "Gamepads") }).mount();
