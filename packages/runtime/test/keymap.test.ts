import { expect, jest, test } from "bun:test";
import type { HotkeyBinding, HotkeyEvent } from "../src/hotkeys";
import { kbdPresentation } from "../src/kbd";
import { defineKeymap, type SkKeyMapEvent } from "../src/keymap";

function override(name: string, value: unknown) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, name);
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
  return () => {
    if (previous) Object.defineProperty(globalThis, name, previous);
    else Reflect.deleteProperty(globalThis, name);
  };
}

test("one immutable declaration supplies localized legends without registering a command", () => {
  const restore = override("__sk_keyLegends", { KeyQ: "й", KeyA: "ф" });
  try {
    const source = ["Control", { code: "KeyQ" }];
    const map = defineKeymap({ keys: source });
    source[1] = { code: "KeyA" };
    expect(map.scope).toBe("app");
    expect(kbdPresentation({ map }).label).toBe("Control plus Й");
    expect(kbdPresentation({ keys: "Control+Q" }).label).toBe("Control plus Q");
    expect(kbdPresentation({ map, labels: { keys: { KeyQ: "Custom" } } }).label).toBe("Control plus Custom");
    expect(kbdPresentation({ keys: { buttons: [{ device: 0, code: 0x51 }] } }).label).toBe("Й");
  } finally {
    restore();
  }
});

test("global commands keep independent lifetimes, skip releases and roll back refused registrations", () => {
  let installed: readonly HotkeyBinding[] = [];
  const listeners = new Set<(event: HotkeyEvent) => void>();
  let refuse = false;
  let running = true;
  const cleanup = [
    override("__sk_keyScans", { KeyQ: 0x10 }),
    override("__sk_appHotkeys", {
      available: true,
      status: () => ({ running, accessibilityGranted: true }),
      _setKeymapBindings(next: readonly HotkeyBinding[]) {
        if (refuse) throw new Error("refused");
        installed = next;
      },
      onFired(listener: (event: HotkeyEvent) => void) {
        listeners.add(listener);
        return () => listeners.delete(listener);
      },
    }),
  ];
  try {
    let activations = 0;
    const command = defineKeymap({ keys: "Ctrl+KeyQ", scope: "global", suppress: true });
    expect(installed).toHaveLength(0);
    const off = command.bind(() => activations++);
    const mouse = defineKeymap({ keys: { mouse: "back" }, scope: "global" }).bind(() => {});
    expect(installed).toHaveLength(2);
    expect(installed[0]!.chord.buttons).toEqual([
      { device: 0, code: 0x30011 },
      { device: 0, code: 0x20010 },
    ]);
    expect(installed[0]!.suppress).toBe(true);
    for (const phase of ["down", "up"] as const)
      for (const listener of listeners) listener({ id: installed[0]!.id, phase });
    expect(activations).toBe(1);
    refuse = true;
    const extra = command.on("up", () => {});
    expect(() => command.disable()).toThrow("refused");
    expect(command.enabled).toBe(true);
    expect(() => defineKeymap({ keys: { mouse: "forward" }, scope: "global" }).bind(() => {})).toThrow("refused");
    expect(listeners.size).toBe(2);
    refuse = false;
    extra();
    off();
    off();
    expect(installed).toHaveLength(1);
    expect(installed[0]!.chord.buttons).toEqual([{ device: 1, code: 4 }]);
    mouse();
    expect(installed).toHaveLength(0);
    expect(listeners.size).toBe(0);
    running = false;
    expect(() => command.bind(() => {})).toThrow("stopped");
    expect(installed).toHaveLength(0);
    expect(listeners.size).toBe(0);
  } finally {
    for (const reset of cleanup) reset();
  }
});

function localInput() {
  const doc = new EventTarget();
  const win = new EventTarget();
  const restore = [
    override("document", doc),
    override("addEventListener", win.addEventListener.bind(win)),
    override("removeEventListener", win.removeEventListener.bind(win)),
  ];
  return {
    send(type: string, fields: Record<string, unknown> = {}) {
      const event = Object.assign(new Event(type, { cancelable: true }), fields);
      doc.dispatchEvent(event);
      return event;
    },
    blur: () => win.dispatchEvent(new Event("blur")),
    restore: () => restore.forEach((reset) => reset()),
  };
}

type TestPad = { id: number; mapping: string };
type PadChange = { type: string; gamepad: TestPad; index?: number; edge?: "down" | "up" | null };
function localGamepads(...pads: TestPad[]) {
  const listeners = new Set<(e: PadChange) => void>();
  const restore = override("__sk_std", {
    gamepads: {
      available: true,
      list: () => pads,
      subscribe(fn: (e: PadChange) => void) {
        listeners.add(fn);
        return () => listeners.delete(fn);
      },
    },
  });
  return { restore, listeners, send: (e: PadChange) => listeners.forEach((fn) => fn(e)) };
}

test("presses, exclusive clicks, double clicks and holds share one map with keyboard and mouse alternatives", () => {
  jest.useFakeTimers();
  const input = localInput();
  const map = defineKeymap({ keys: { alternatives: ["F2", { mouse: "back" }] }, holdMs: 80, doubleClickMs: 40 });
  const events: string[] = [];
  const off = (["down", "up", "press", "click", "doubleClick", "hold"] as const).map((type) =>
    map.on(type, (e) => {
      expect(e.map).toBe(map);
      events.push(e.type + e.alternative);
    }),
  );
  const key = (edge: string) => input.send("key" + edge, { key: "F2", code: "F2" });
  const mouse = (edge: string) => input.send("mouse" + edge, { button: 3 });
  try {
    key("down");
    input.send("keydown", { key: "F2", code: "F2", repeat: true });
    key("up");
    expect(events.splice(0)).toEqual(["down0", "up0", "press0"]);
    jest.advanceTimersByTime(40);
    expect(events.splice(0)).toEqual(["click0"]);
    mouse("down");
    mouse("up");
    jest.advanceTimersByTime(20);
    mouse("down");
    mouse("up");
    jest.advanceTimersByTime(100);
    expect(events.splice(0)).toEqual(["down1", "up1", "press1", "down1", "up1", "press1", "doubleClick1"]);
    key("down");
    key("up");
    mouse("down");
    mouse("up");
    jest.advanceTimersByTime(40);
    expect(events.splice(0)).toEqual(["down0", "up0", "press0", "down1", "up1", "press1", "click0", "click1"]);
    mouse("down");
    jest.advanceTimersByTime(200);
    mouse("up");
    jest.advanceTimersByTime(100);
    expect(events).toEqual(["down1", "hold1", "up1", "press1"]);
  } finally {
    off.forEach((stop) => stop());
    input.restore();
    jest.useRealTimers();
  }
});

test("app gamepad chords use one physical pad, share a single subscription and cancel on removal", () => {
  const input = localInput();
  const pad = (id: number, mapping = "standard") => ({ id, mapping, buttonValues: [0, 0] });
  const first = pad(1),
    second = pad(2),
    raw = pad(3, "");
  const { listeners, send, restore } = localGamepads(first, second, raw);
  const edge = (gamepad: typeof first, index: number, pressed: boolean) =>
    send({ type: "button", gamepad, index, edge: pressed ? "down" : "up" });
  const seen: string[] = [];
  const map = defineKeymap({ keys: [{ gamepad: "south" }, { gamepad: "east" }] });
  const stops: (() => void)[] = [];
  try {
    stops.push(
      map.bind(() => seen.push("down")),
      map.on("up", (e) => seen.push(e.cancelled ? "cancel" : "up")),
    );
    stops.push(defineKeymap({ keys: { gamepad: 0 } }).bind(() => seen.push("raw0")));
    expect(listeners.size).toBe(1);
    edge(first, 0, true);
    edge(second, 1, true);
    edge(first, 0, true);
    expect(seen.splice(0)).toEqual(["raw0"]);
    edge(first, 1, true);
    edge(second, 0, true);
    edge(first, 0, false);
    expect(seen.splice(0)).toEqual(["down"]);
    send({ type: "disconnected", gamepad: second });
    expect(seen.splice(0)).toEqual(["cancel"]);
    edge(raw, 0, true);
    edge(raw, 1, true);
    expect(seen.splice(0)).toEqual(["raw0"]);
    send({ type: "connected", gamepad: second });
    edge(second, 1, true);
    edge(second, 0, true);
    edge(second, 1, false);
    expect(seen).toEqual(["down", "up"]);
    const sequence = defineKeymap({ keys: { sequence: [{ gamepad: "dpad-up" }, { gamepad: "dpad-down" }] } });
    let sequences = 0;
    stops.push(sequence.bind(() => sequences++));
    edge(first, 12, true);
    edge(first, 12, false);
    edge(second, 13, true);
    edge(second, 13, false);
    expect(sequences).toBe(0);
    edge(first, 13, true);
    expect(sequences).toBe(1);
  } finally {
    stops.forEach((off) => off());
    expect(listeners.size).toBe(0);
    restore();
    input.restore();
  }
});

test("a keyboard modifier can complete a gamepad chord, and blur cancels it", () => {
  const input = localInput();
  const gamepad = { id: 1, mapping: "standard", buttonValues: [1] };
  const { send, restore } = localGamepads(gamepad);
  const edge = (value: "down" | "up" | null) => send({ type: "button", gamepad, index: 0, edge: value });
  const seen: string[] = [];
  const map = defineKeymap({ keys: ["Shift", { gamepad: "south" }] });
  const stops = [map.bind(() => seen.push("down")), map.on("up", (e) => seen.push(e.cancelled ? "cancel" : "up"))];
  try {
    input.send("keydown", { key: "Shift", code: "ShiftLeft", shiftKey: true });
    edge(null);
    expect(seen).toEqual([]); // Already held when the command subscribed.
    edge("up");
    edge("down");
    expect(seen.splice(0)).toEqual(["down"]);
    input.blur();
    edge("up");
    edge("down");
    expect(seen.splice(0)).toEqual(["cancel"]);
    input.send("keydown", { key: "Shift", code: "ShiftRight", shiftKey: true });
    expect(seen).toEqual(["down"]);
  } finally {
    stops.forEach((off) => off());
    restore();
    input.restore();
  }
});

test("without a gamepad service a mixed map still uses its keyboard alternative", () => {
  const input = localInput();
  const restore = override("__sk_std", { gamepads: { available: false } });
  const stopped: (() => void)[] = [];
  try {
    const hits: number[] = [];
    stopped.push(
      defineKeymap({ keys: { alternatives: [{ gamepad: "south" }, "Enter"] } }).bind((e) => hits.push(e.alternative)),
    );
    input.send("keydown", { key: "Enter", code: "Enter" });
    expect(hits).toEqual([1]);
    expect(() => defineKeymap({ keys: { gamepad: "south" } }).bind(() => {})).toThrow("Gamepad input is unavailable");
  } finally {
    stopped.forEach((off) => off());
    restore();
    input.restore();
  }
});

test("mixed chord clicks belong to their controller even when Enter completes and releases the chord", () => {
  jest.useFakeTimers();
  const input = localInput();
  const first = { id: 1, mapping: "standard" };
  const second = { id: 2, mapping: "standard" };
  const { send, restore } = localGamepads(first, second);
  const enter = () => {
    input.send("keydown", { key: "Enter", code: "Enter" });
    input.send("keyup", { key: "Enter", code: "Enter" });
  };
  const click = (gamepad: typeof first) => {
    send({ type: "button", gamepad, index: 0, edge: "down" });
    enter();
    send({ type: "button", gamepad, index: 0, edge: "up" });
  };
  const seen: string[] = [];
  const map = defineKeymap({ keys: [{ gamepad: "south" }, "Enter"], doubleClickMs: 40 });
  const stops = (["press", "click", "doubleClick"] as const).map((kind) => map.on(kind, (e) => seen.push(e.type)));
  try {
    send({ type: "button", gamepad: first, index: 0, edge: "down" });
    enter();
    send({ type: "disconnected", gamepad: first });
    jest.advanceTimersByTime(40);
    expect(seen.splice(0)).toEqual(["press"]);

    send({ type: "connected", gamepad: first });
    click(first);
    click(second);
    jest.advanceTimersByTime(40);
    expect(seen.splice(0)).toEqual(["press", "press", "click", "click"]);

    click(second);
    send({ type: "disconnected", gamepad: first });
    click(second);
    expect(seen.splice(0)).toEqual(["press", "press", "doubleClick"]);

    send({ type: "connected", gamepad: first });
    send({ type: "button", gamepad: second, index: 0, edge: "down" });
    send({ type: "button", gamepad: first, index: 0, edge: "down" });
    input.send("keydown", { key: "Enter", code: "Enter" });
    send({ type: "disconnected", gamepad: second });
    input.send("keyup", { key: "Enter", code: "Enter" });
    jest.advanceTimersByTime(40);
    expect(seen).toEqual(["press"]);
  } finally {
    stops.forEach((off) => off());
    restore();
    input.restore();
    jest.useRealTimers();
  }
});

test("disable, focus loss, disposal and conditions cancel pending gestures and balance a down", () => {
  jest.useFakeTimers();
  const input = localInput();
  let allowed = false;
  const map = defineKeymap({ keys: "Control+KeyK", enabled: false, suppress: true, when: () => allowed });
  const events: SkKeyMapEvent[] = [];
  const off = (["down", "up", "press", "click", "doubleClick", "hold"] as const).map((type) =>
    map.on(type, (event) => events.push(event)),
  );
  const key = (type: string) => input.send(type, { key: "k", code: "KeyK", ctrlKey: true });
  try {
    expect(key("keydown").defaultPrevented).toBe(false);
    map.enable();
    map.enable();
    expect(key("keydown").defaultPrevented).toBe(false);
    key("keyup");
    allowed = true;
    expect(key("keydown").defaultPrevented).toBe(true);
    map.disable();
    map.disable();
    map.enable();
    key("keyup");
    expect(events.map((e) => [e.type, e.cancelled])).toEqual([
      ["down", false],
      ["up", true],
    ]);
    events.length = 0;
    key("keydown");
    input.blur();
    key("keyup");
    expect(events.map((e) => e.type)).toEqual(["down", "up"]);
    expect(events.pop()?.cancelled).toBe(true);
    events.length = 0;
    key("keydown");
    allowed = false;
    key("keyup");
    expect(events.map((e) => e.cancelled)).toEqual([false, true]);
    events.length = 0;
    allowed = true;
    key("keydown");
    key("keyup");
    allowed = false;
    jest.advanceTimersByTime(501);
    expect(events.map((e) => e.type)).toEqual(["down", "up", "press"]);
    events.length = 0;
    allowed = true;
    key("keydown");
    key("keyup");
    map.disable();
    map.enable();
    jest.advanceTimersByTime(501);
    expect(events.map((e) => e.type)).toEqual(["down", "up", "press"]);
    events.length = 0;
    let once = 0;
    map.on("down", () => once++, { once: true });
    key("keydown");
    key("keyup");
    key("keydown");
    expect(once).toBe(1);
    off.forEach((stop) => stop());
    jest.advanceTimersByTime(1000);
    expect(events.filter((e) => e.type === "hold" || e.type === "click")).toHaveLength(0);
    expect(events[0]?.originalEvent?.type).toBe("keydown");
  } finally {
    off.forEach((stop) => stop());
    input.restore();
    jest.useRealTimers();
  }
});

test("the final chord in a sequence releases on either member, and a wheel step completes immediately", () => {
  const input = localInput();
  const map = defineKeymap({
    keys: { alternatives: [{ sequence: ["g", ["Control", "KeyK"]] }, { mouse: "wheel-up" }] },
  });
  const events: string[] = [];
  const off = (["down", "up", "click"] as const).map((type) =>
    map.on(type, (e) => events.push(e.type + e.alternative)),
  );
  try {
    input.send("keydown", { key: "g", code: "KeyG" });
    input.send("keyup", { key: "g", code: "KeyG" });
    input.send("keydown", { key: "k", code: "KeyK", ctrlKey: true });
    input.send("keyup", { key: "Control", code: "ControlLeft" });
    input.send("keyup", { key: "k", code: "KeyK" });
    input.send("wheel", { deltaX: 0, deltaY: -5 });
    expect(events).toEqual(["down0", "up0", "click0", "down1", "up1", "click1"]);
  } finally {
    off.forEach((stop) => stop());
    input.restore();
  }
});

test("overlapping alternatives form one hold and dragging does not produce a click", () => {
  const input = localInput();
  const map = defineKeymap({ keys: { alternatives: ["F2", { mouse: "back" }] } });
  const events: string[] = [];
  const off = (["down", "up", "click"] as const).map((type) =>
    map.on(type, (e) => events.push(e.type + e.alternative)),
  );
  try {
    input.send("keydown", { key: "F2", code: "F2" });
    input.send("mousedown", { button: 3 });
    input.send("keyup", { key: "F2", code: "F2" });
    expect(events).toEqual(["down0"]);
    input.send("mouseup", { button: 3 });
    expect(events.splice(0)).toEqual(["down0", "up0", "click0"]);
    input.send("mousedown", { button: 3, clientX: 10, clientY: 10 });
    input.send("mouseup", { button: 3, clientX: 60, clientY: 10 });
    expect(events).toEqual(["down1", "up1"]);
  } finally {
    off.forEach((stop) => stop());
    input.restore();
  }
});

test("per-listener conditions arbitrate double clicks and a one-shot waits for a qualifying press", () => {
  jest.useFakeTimers();
  const input = localInput();
  const map = defineKeymap({ keys: "F2" });
  let editing = false;
  const calls: string[] = [];
  const off = [
    map.on("click", () => calls.push("single")),
    map.on("doubleClick", () => calls.push("double"), { when: () => editing }),
    map.on("press", () => calls.push("once"), { when: (e) => editing && e.alternative === 0, once: true }),
  ];
  const press = () => {
    input.send("keydown", { key: "F2", code: "F2" });
    input.send("keyup", { key: "F2", code: "F2" });
  };
  try {
    press();
    expect(calls.splice(0)).toEqual(["single"]);
    editing = true;
    press();
    press();
    expect(calls.splice(0)).toEqual(["once", "double"]);
    press();
    editing = false;
    press();
    jest.advanceTimersByTime(500);
    expect(calls).toEqual(["single", "single"]);
  } finally {
    off.forEach((stop) => stop());
    input.restore();
    jest.useRealTimers();
  }
});

test("global gestures share a registration and distinguish interrupted and physical releases", () => {
  jest.useFakeTimers();
  let bindings: readonly HotkeyBinding[] = [];
  const native = new Set<(event: HotkeyEvent) => void>();
  const restore = override("__sk_appHotkeys", {
    available: true,
    status: () => ({ running: true, accessibilityGranted: true }),
    _setKeymapBindings(next: readonly HotkeyBinding[]) {
      bindings = next;
    },
    onFired(handler: (event: HotkeyEvent) => void) {
      native.add(handler);
      return () => native.delete(handler);
    },
  });
  let allowed = true;
  const map = defineKeymap({ keys: { mouse: "back" }, scope: "global", when: () => allowed });
  const events: string[] = [];
  const off = (["down", "up", "click", "doubleClick"] as const).map((type) =>
    map.on(type, (e) => {
      expect(e.originalEvent).toBeNull();
      events.push(e.cancelled ? "cancelled" : e.type);
    }),
  );
  const fire = (phase: HotkeyEvent["phase"], cancelled = false) => {
    for (const handler of native) handler({ id: bindings[0]!.id, phase, ...(cancelled ? { cancelled: true } : {}) });
  };
  try {
    expect(native.size).toBe(1);
    expect(bindings).toHaveLength(1);
    fire("down");
    fire("up", true);
    expect(events.splice(0)).toEqual(["down", "cancelled"]);
    allowed = false;
    fire("down");
    fire("up");
    expect(events).toEqual([]);
    allowed = true;
    fire("down");
    fire("down");
    fire("up");
    fire("down");
    fire("up");
    expect(events.splice(0)).toEqual(["down", "up", "down", "up", "doubleClick"]);
    fire("down");
    map.disable();
    expect(bindings).toHaveLength(0);
    expect(native.size).toBe(0);
    expect(events.splice(0)).toEqual(["down", "cancelled"]);
    map.enable();
    fire("down");
    fire("up");
    jest.advanceTimersByTime(500);
    expect(events).toEqual(["down", "up", "click"]);
  } finally {
    off.forEach((stop) => stop());
    restore();
    jest.useRealTimers();
  }
});

test("app keymaps distinguish physical and literal keys, ignore repeats and clear sequences on focus loss", () => {
  const document = new EventTarget(),
    window = new EventTarget();
  const restore = [
    override("document", document),
    override("addEventListener", window.addEventListener.bind(window)),
    override("removeEventListener", window.removeEventListener.bind(window)),
  ];
  const stopped: (() => void)[] = [];
  try {
    const fired: string[] = [];
    for (const [id, keys] of [
      ["physical", "Control+KeyQ"],
      ["logical", "Control+Й"],
      ["sequence", { sequence: ["g", "g"] }],
    ] as const) {
      stopped.push(defineKeymap({ keys, suppress: true }).bind(() => fired.push(id)));
    }
    function send(type: string, key: string, code: string, extra: Record<string, unknown> = {}) {
      const e = Object.assign(new Event(type, { cancelable: true }), { key, code, ...extra });
      document.dispatchEvent(e);
      return e.defaultPrevented;
    }
    expect(send("keydown", "й", "KeyQ", { ctrlKey: true })).toBe(true);
    send("keydown", "й", "KeyQ", { ctrlKey: true, repeat: true });
    send("keyup", "й", "KeyQ");
    send("keydown", "й", "KeyF", { ctrlKey: true });
    send("keyup", "й", "KeyF");
    send("keydown", "й", "KeyQ", { ctrlKey: true, isComposing: true });
    expect(fired).toEqual(["physical", "logical", "logical"]);
    const g = () => {
      send("keydown", "g", "KeyG");
      send("keyup", "g", "KeyG");
    };
    g();
    window.dispatchEvent(new Event("blur"));
    g();
    expect(fired).toHaveLength(3);
    g();
    expect(fired.at(-1)).toBe("sequence");
    for (const stop of stopped) stop();
    send("keydown", "й", "KeyQ", { ctrlKey: true });
    expect(fired).toHaveLength(4);
  } finally {
    for (const stop of stopped) stop();
    for (const reset of restore) reset();
  }
});
