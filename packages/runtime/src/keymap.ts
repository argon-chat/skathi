import { buttonName, canonical, chordTokens, platformOf, type SkKbdChord, type SkKbdKeys } from "./kbd";
import { capturedPosition, keyScan, onKeyLayoutChange } from "./keyboard-layout";
import { windowInputLoss, windowRoots } from "./skwindow";
import type { AppHotkeys, HotkeyBinding, HotkeyButton } from "./hotkeys";
import {
  KeymapGestures,
  type KeymapInput,
  type KeymapListener,
  type SkKeyMapEventType,
  type SkKeyMapHandler,
  type SkKeyMapListenerOptions,
} from "./keymap-gestures";
export type { SkKeyMapEvent, SkKeyMapEventType, SkKeyMapHandler, SkKeyMapListenerOptions } from "./keymap-gestures";

export interface SkKeyMapOptions {
  readonly keys: SkKbdKeys;
  readonly scope?: "app" | "global";
  readonly mousePrimary?: "left" | "right";
  /** Consume a matching event. Global bindings pass it through by default. */
  readonly suppress?: boolean;
  /** Minimum interval between global activations, in milliseconds. */
  readonly triggerCooldownMs?: number;
  /** Allow bare app shortcuts while editing text. */
  readonly allowInEditable?: boolean;
  /** Set false to start inactive and retain subscriptions until enabled. */
  readonly enabled?: boolean;
  /** Checked before matching and before completing a gesture. Global input has no DOM event. */
  readonly when?: (event: KeymapInput) => boolean;
  /** Milliseconds before hold and between releases in a double click. Both default to 500. */
  readonly holdMs?: number;
  readonly doubleClickMs?: number;
}
export interface SkKeyMap {
  readonly keys: SkKbdKeys;
  readonly scope: "app" | "global";
  readonly mousePrimary?: "left" | "right";
  readonly enabled: boolean;
  enable(): void;
  disable(): void;
  /** Subscribe to down. Rendering a map never registers it. */
  bind(handler: SkKeyMapHandler): () => void;
  on(type: SkKeyMapEventType, handler: SkKeyMapHandler, options?: SkKeyMapListenerOptions): () => void;
}

const stepsOf = (keys: SkKbdKeys): readonly (readonly SkKbdChord[])[] =>
  (typeof keys === "object" && "alternatives" in keys ? keys.alternatives : [keys]).map((choice) =>
    typeof choice === "object" && "sequence" in choice ? choice.sequence : [choice],
  );
const modifiers: Readonly<Record<string, number>> = { Control: 0x11, Alt: 0x12, Shift: 0x10, Meta: 0x5b };
const sides: Readonly<Record<string, number>> = {
  ControlLeft: 0xa2,
  ControlRight: 0xa3,
  AltLeft: 0xa4,
  AltRight: 0xa5,
  ShiftLeft: 0xa0,
  ShiftRight: 0xa1,
  MetaLeft: 0x5b,
  MetaRight: 0x5c,
};
const mouseIds = [
  ["MousePrimary", "Mouse1"],
  ["Mouse3"],
  ["MouseSecondary", "Mouse2"],
  ["MouseBack", "Mouse4"],
  ["MouseForward", "Mouse5"],
] as const;
const modifierOf = (name: string) => name.replace(/(Left|Right)$/, "");
const apple = () => platformOf() === "macos";
const literal = (name: string) => ([...name].length === 1 ? name.toLowerCase() : name);

function localStep(chord: SkKbdChord, primary: SkKeyMapOptions["mousePrimary"]): string[] {
  const keys =
    typeof chord === "object" && "buttons" in chord
      ? chord.buttons.map((button) => capturedPosition(button) ?? buttonName(button))
      : chordTokens(chord).map((token) => literal(canonical(token, apple())));
  return keys.map((key) =>
    key === "Mouse1"
      ? primary === "right"
        ? "MouseSecondary"
        : "MousePrimary"
      : key === "Mouse2"
        ? primary === "right"
          ? "MousePrimary"
          : "MouseSecondary"
        : key,
  );
}
type Choice = { alternative: number; steps: string[][]; index: number; last: number };
type Local = { choices: Choice[]; active: Set<Choice>; gestures: KeymapGestures; suppress: boolean; edit: boolean };
const locals = new Set<Local>();
const held = new Map<string, readonly string[]>();
let inputWindow: object | undefined;

function resetLocal(local: Local) {
  local.active.clear();
  for (const choice of local.choices) choice.index = 0;
  local.gestures.reset();
}
function resetInput() {
  inputWindow = undefined;
  held.clear();
  for (const local of [...locals]) resetLocal(local);
}
function windowLost(root?: object) {
  if (inputWindow === root) resetInput();
}
const mainBlur = () => windowLost();
function localEvent(event: Event) {
  const e = event as KeyboardEvent & MouseEvent & WheelEvent;
  const isKey = e.type.startsWith("key");
  const wheel = e.type === "wheel";
  const id = isKey ? e.code || e.key : "mouse:" + e.button;
  const owner = windowRoots.size
    ? e.composedPath().find((n): n is EventTarget => !!n && typeof n === "object" && windowRoots.has(n))
    : undefined;
  if (owner !== inputWindow) {
    resetInput();
    inputWindow = owner;
  }
  const up = e.type === "keyup" || e.type === "mouseup";
  if (!up && (e.defaultPrevented || (isKey && (e.repeat || e.isComposing || e.key === "Process" || e.key === "Dead"))))
    return;
  if (!up && !wheel && held.has(id)) return;
  let names: readonly string[];
  if (isKey) {
    const key = canonical(e.key, apple());
    names = [e.code, literal(key), ...(e.location === 3 ? ["Numpad" + key] : [])].filter(Boolean);
  } else if (wheel) {
    if (!e.deltaX && !e.deltaY) return;
    names = [
      Math.abs(e.deltaX) > Math.abs(e.deltaY)
        ? e.deltaX > 0
          ? "WheelRight"
          : "WheelLeft"
        : e.deltaY > 0
          ? "WheelDown"
          : "WheelUp",
    ];
  } else names = mouseIds[e.button] ?? ["Mouse" + (e.button + 1)];
  const flags = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Meta"].filter(
    (s): s is string => !!s,
  );
  if (up) {
    names = held.get(id) ?? names;
    held.delete(id);
    const remaining = new Set([...flags, ...Array.from(held.values()).flat()]);
    const cancelled = e.defaultPrevented || (isKey && e.isComposing);
    for (const local of [...locals]) {
      for (const choice of [...local.active]) {
        if (!choice.steps.at(-1)!.some((key) => names.includes(key) && !remaining.has(key))) continue;
        local.active.delete(choice);
        if (local.suppress) e.preventDefault();
        local.gestures.up(choice.alternative, e, cancelled);
      }
    }
    return;
  }
  if (!wheel) held.set(id, names);
  const editing = !!(e.target as { closest?: (selector: string) => unknown } | null)?.closest?.(
    "input, textarea, [contenteditable='true'], [contenteditable=''], [role='textbox']",
  );
  const active = new Set([...flags, ...names, ...Array.from(held.values()).flat()]);
  function matches(step: readonly string[]) {
    return (
      step.length > 0 &&
      step.some((key) => names.includes(key)) &&
      step.every((key) => (Object.hasOwn(modifiers, key) ? flags.includes(key) : active.has(key))) &&
      flags.every((mod) => step.some((key) => modifierOf(key) === mod))
    );
  }
  const now = Date.now();
  for (const local of [...locals]) {
    if (!locals.has(local)) continue;
    if (
      (editing && !local.edit && ((!e.ctrlKey && !e.metaKey && !e.altKey) || e.getModifierState?.("AltGraph"))) ||
      !local.gestures.allows(e)
    ) {
      resetLocal(local);
      continue;
    }
    if (!locals.has(local)) continue;
    let completed: Choice | undefined;
    for (const choice of local.choices) {
      if (local.active.has(choice)) continue;
      if (now - choice.last > 1000) choice.index = 0;
      if (!matches(choice.steps[choice.index]!)) {
        if (names.some((name) => Object.hasOwn(modifiers, modifierOf(name)))) continue;
        choice.index = 0;
        if (!matches(choice.steps[0]!)) continue;
      }
      choice.last = now;
      if (local.suppress) e.preventDefault();
      if (++choice.index === choice.steps.length) {
        choice.index = 0;
        completed ??= choice;
      }
    }
    if (completed) {
      for (const choice of local.choices) choice.index = 0;
      local.active.add(completed);
      local.gestures.down(completed.alternative, e);
      if (wheel && local.active.delete(completed)) {
        local.gestures.up(completed.alternative, e);
      }
    }
  }
}
const events = ["keydown", "keyup", "mousedown", "mouseup", "wheel"] as const;
const releaseEvent = (type: string) => type === "keyup" || type === "mouseup";
const appWindow = globalThis as typeof globalThis & EventTarget;
function bindLocal(options: SkKeyMapOptions, gestures: KeymapGestures) {
  const source = stepsOf(options.keys);
  const compile = () =>
    source.map((steps, alternative) => ({
      alternative,
      steps: steps.map((step) => localStep(step, options.mousePrimary)),
      index: 0,
      last: 0,
    }));
  const choices = compile();
  if (!choices.length || choices.some((c) => !c.steps.length || c.steps.some((s) => !s.length || s.includes(""))))
    throw new TypeError("An app keymap needs a nonempty shortcut.");
  const canBind = (c: Choice) => !c.steps.some((s) => s.some((k) => /^Gamepad[:B]/.test(k)));
  const supported = choices.filter(canBind);
  if (!supported.length)
    throw new Error("Gamepad input is unavailable. Supply a keyboard or mouse alternative to activate this map.");
  const local: Local = {
    choices: supported,
    active: new Set(),
    gestures,
    suppress: options.suppress ?? false,
    edit: options.allowInEditable ?? false,
  };
  if (!locals.size) {
    for (const type of events)
      document.addEventListener(type, localEvent, type === "wheel" ? { passive: false } : releaseEvent(type));
    appWindow.addEventListener("blur", mainBlur);
    document.addEventListener("pointercancel", resetInput);
    windowInputLoss.add(windowLost);
  }
  locals.add(local);
  const offLayout = source.some((steps) => steps.some((step) => typeof step === "object" && "buttons" in step))
    ? onKeyLayoutChange(() => {
        local.choices = compile().filter(canBind);
        resetInput();
      })
    : undefined;
  return () => {
    offLayout?.();
    if (!locals.delete(local) || locals.size) return;
    for (const type of events) document.removeEventListener(type, localEvent, releaseEvent(type));
    appWindow.removeEventListener("blur", mainBlur);
    document.removeEventListener("pointercancel", resetInput);
    windowInputLoss.delete(windowLost);
    resetInput();
  };
}

type GlobalHost = AppHotkeys & { _setKeymapBindings(bindings: readonly HotkeyBinding[]): boolean };
const globalBindings = new Map<string, HotkeyBinding>();
let nextId = 0;
function globalButton(name: string, primary: SkKeyMapOptions["mousePrimary"]): HotkeyButton {
  if (Object.hasOwn(modifiers, name)) return { device: 0, code: 0x30000 | modifiers[name]! };
  if (Object.hasOwn(sides, name)) return { device: 0, code: sides[name]! };
  if (name === "MousePrimary" || name === "MouseSecondary")
    return { device: 1, code: (name === "MousePrimary") === (primary !== "right") ? 1 : 2 };
  const mouse = /^Mouse([1-5])$/.exec(name);
  if (mouse || name === "MouseBack" || name === "MouseForward")
    return { device: 1, code: mouse ? Number(mouse[1]) : name === "MouseBack" ? 4 : 5 };
  const scan = keyScan(name);
  if (typeof scan === "number" && scan) return { device: 0, code: 0x20000 | scan };
  throw new TypeError(
    "Global keymaps need physical keyboard codes (such as KeyQ) or mouse buttons. Unsupported key: " + name,
  );
}
function bindGlobal(options: SkKeyMapOptions, gestures: KeymapGestures) {
  const host = (globalThis as typeof globalThis & { __sk_appHotkeys?: GlobalHost }).__sk_appHotkeys;
  if (!host?.available) throw new Error("Global hotkeys are unavailable on this host.");
  if (!host.status().accessibilityGranted)
    throw new Error(
      "Global hotkeys need Accessibility access. Grant it to the app in System Settings, then bind again.",
    );
  const choices = stepsOf(options.keys);
  if (!choices.length) throw new TypeError("A global keymap needs a shortcut.");
  const bindings = choices.map((steps): HotkeyBinding => {
    if (steps.length !== 1)
      throw new TypeError("Global hotkeys accept simultaneous chords. Use app scope for sequences.");
    const step = steps[0]!;
    const buttons =
      typeof step === "object" && "buttons" in step
        ? step.buttons
        : chordTokens(step).map((k) => globalButton(canonical(k, apple()), options.mousePrimary));
    if (!buttons.length) throw new TypeError("A global keymap needs a nonempty chord.");
    return {
      id: "@sk/keymap/" + ++nextId,
      chord: { buttons },
      suppress: options.suppress ?? false,
      ...(options.triggerCooldownMs === undefined ? {} : { triggerCooldownMs: options.triggerCooldownMs }),
    };
  });
  const alternatives = new Map(bindings.map((binding, index) => [binding.id, index]));
  const off = host.onFired((e) => {
    const index = alternatives.get(e.id);
    if (index === undefined) return;
    if (e.phase === "up") gestures.up(index, null, e.cancelled);
    else if (gestures.allows(null)) gestures.down(index, null);
  });
  for (const binding of bindings) globalBindings.set(binding.id, binding);
  let installed = false;
  try {
    host._setKeymapBindings([...globalBindings.values()]);
    installed = true;
    const status = host.status();
    if (!status.running)
      throw new Error(status.error ?? "Global hotkeys stopped. Call appHotkeys.restart() and bind again.");
  } catch (error) {
    off();
    for (const id of alternatives.keys()) globalBindings.delete(id);
    if (installed) host._setKeymapBindings([...globalBindings.values()]);
    throw error;
  }
  return () => {
    for (const id of alternatives.keys()) globalBindings.delete(id);
    try {
      host._setKeymapBindings([...globalBindings.values()]);
    } catch (error) {
      for (const binding of bindings) globalBindings.set(binding.id, binding);
      throw error;
    }
    off();
  };
}

function snapshot<T>(value: T): T {
  if (!value || typeof value !== "object") return value;
  const copy = Array.isArray(value)
    ? value.map(snapshot)
    : Object.fromEntries(Object.entries(value).map(([name, entry]) => [name, snapshot(entry)]));
  return Object.freeze(copy) as T;
}

/** Define one command's shortcut. The first subscription activates an enabled map. */
export function defineKeymap({ keys, scope = "app", ...settings }: SkKeyMapOptions): SkKeyMap {
  keys = snapshot(keys);
  const options = { keys, scope, ...settings };
  let enabled = settings.enabled ?? true;
  let stop: (() => void) | undefined;
  let syncing = false;
  function sync() {
    if (syncing) return;
    syncing = true;
    try {
      while (!!stop !== (enabled && gestures.listeners.size > 0)) {
        if (stop) {
          stop();
          stop = undefined;
          gestures.reset();
        } else stop = scope === "global" ? bindGlobal(options, gestures) : bindLocal(options, gestures);
      }
    } finally {
      syncing = false;
    }
  }
  function setEnabled(value: boolean) {
    if (enabled === value) return;
    enabled = value;
    try {
      sync();
    } catch (error) {
      enabled = !value;
      throw error;
    }
  }
  const map: SkKeyMap = {
    keys,
    scope,
    ...(settings.mousePrimary ? { mousePrimary: settings.mousePrimary } : {}),
    get enabled() {
      return enabled;
    },
    enable: () => setEnabled(true),
    disable: () => setEnabled(false),
    bind: (handler) => map.on("down", handler),
    on(type, handler, { once = false, when } = {}) {
      const off = () => {
        if (!gestures.listeners.delete(listener)) return;
        try {
          sync();
        } catch (error) {
          gestures.listeners.add(listener);
          throw error;
        }
      };
      const listener: KeymapListener = {
        type,
        ...(when && { when }),
        handler: once
          ? (event) => {
              off();
              handler(event);
            }
          : handler,
      };
      gestures.listeners.add(listener);
      try {
        sync();
      } catch (error) {
        gestures.listeners.delete(listener);
        throw error;
      }
      return off;
    },
  };
  const gestures = new KeymapGestures(map, options);
  return Object.freeze(map);
}
