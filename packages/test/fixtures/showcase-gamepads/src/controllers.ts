import type { Controller, ControllerEvent, DeadZoneTarget, Gamepads } from "skathi:gamepad";
export type { Controller, ControllerEvent, DeadZoneTarget } from "skathi:gamepad";

function device(
  id: number,
  index: number,
  name: string,
  family: Controller["family"],
  vendorId: number,
  productId: number,
  systemId: string | null,
): Controller {
  return {
    id,
    systemId,
    index,
    name,
    family,
    vendorId,
    productId,
    mapping: "standard",
    timestamp: 10,
    canRumble: true,
    deadZone: 0.05,
    axes: { "left-x": 0.25 },
    axisValues: [0.25, 0, 0, 0],
    buttons: { south: 0 },
    buttonValues: Array(17).fill(0),
  };
}
const pads = [
  device(41, 0, "Xbox fixture", "xbox", 0x045e, 0x028e, null),
  device(92, 2, "DualShock fixture", "playstation", 0x054c, 0x09cc, "fixture-dualshock"),
];
const listeners = new Set<(event: ControllerEvent) => void>();
const commands: unknown[][] = [];
const overrides = new Map<string, number>();
let polls = 0;
let lists = 0;
let deadZoneReads = 0;
function settingKey(target: DeadZoneTarget) {
  if (target.scope === "global") return "global";
  if (target.scope === "model") return "model/" + target.vendorId + "/" + target.productId;
  if ("systemId" in target) return "device/" + target.systemId;
  return "device/" + (pads.find((pad) => pad.id === target.id)?.systemId ?? target.id);
}
export const gamepads: Gamepads = {
  available: true,
  list() {
    lists++;
    return pads;
  },
  subscribe(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  rumble(id, options) {
    commands.push(["native", id, options]);
    return true;
  },
  getDeadZone(target) {
    deadZoneReads++;
    return overrides.get(settingKey(target)) ?? null;
  },
  setDeadZone(target, radius) {
    const key = settingKey(target);
    if (radius === null) overrides.delete(key);
    else overrides.set(key, radius);
    pads.forEach((pad, index) => {
      const device = settingKey({ scope: "device", id: pad.id });
      const model = settingKey({ scope: "model", vendorId: pad.vendorId, productId: pad.productId });
      const deadZone = overrides.get(device) ?? overrides.get(model) ?? overrides.get("global") ?? 0.05;
      pads[index] = { ...pad, deadZone };
    });
  },
};

Object.defineProperty(globalThis, "__sk_std", {
  value: Object.create(Reflect.get(globalThis, "__sk_std"), { gamepads: { value: gamepads } }),
});
Object.defineProperty(navigator, "getGamepads", {
  value: () => {
    polls++;
    const result = Array(3).fill(null);
    for (const pad of pads) {
      result[pad.index] = {
        id: pad.name,
        index: pad.index,
        timestamp: pad.timestamp,
        connected: true,
        mapping: pad.mapping,
        axes: pad.axisValues,
        buttons: pad.buttonValues.map((value) => ({ value, pressed: value > 0.5, touched: value > 0 })),
        vibrationActuator: {
          effects: ["dual-rumble"],
          async playEffect(name: string, options: object) {
            commands.push(["web", pad.index, name, options]);
            return "complete";
          },
          async reset() {
            commands.push(["web", pad.index, "reset"]);
            return "complete";
          },
        },
      };
    }
    return result;
  },
});
Object.assign(globalThis, {
  showcaseGamepadStats: () => ({ listeners: listeners.size, polls, lists, deadZoneReads }),
  showcaseGamepadAxis(value: number) {
    const old = pads[1]!;
    const pad = {
      ...old,
      timestamp: old.timestamp + 10,
      axes: { "left-x": value },
      axisValues: [value, ...old.axisValues.slice(1)],
    };
    pads[1] = pad;
    for (const fn of listeners) fn({ type: "axis", gamepad: pad, index: 0, axis: "left-x", value });
  },
  showcaseGamepadInput(pressed = true) {
    const old = pads[1]!;
    const value = Number(pressed);
    const pad = {
      ...old,
      timestamp: old.timestamp + 10,
      axes: { "left-x": -0.75 },
      axisValues: [-0.75, 0, 0, 0],
      buttons: { south: value },
      buttonValues: [value, ...old.buttonValues.slice(1)],
    };
    pads[1] = pad;
    for (const fn of listeners)
      fn({ type: "button", gamepad: pad, index: 0, button: "south", value, pressed, edge: pressed ? "down" : "up" });
  },
  showcaseGamepadCommands: commands,
});
