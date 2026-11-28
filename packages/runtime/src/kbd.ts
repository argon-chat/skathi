import type { HotkeyButton, HotkeyChord } from "./hotkeys";
import type { SkKeyMap } from "./keymap";
import { capturedPosition, keyboardLegend } from "./keyboard-layout";
import {
  controllerLayout,
  type ControllerIdentity,
  deviceKeycap,
  deviceKeyId,
  type KbdKeycap,
  type SkKbdDeviceInput,
  type SkKbdGamepadLayout,
} from "./kbd-devices";
export type { SkKbdDeviceInput, SkKbdGamepadButton, SkKbdGamepadLayout, SkKbdMouseButton } from "./kbd-devices";

/** An array combines literal keyboard keys, mouse inputs, and controller buttons. */
export type SkKbdKey = string | { readonly code: string } | SkKbdDeviceInput;
export type SkKbdChord = SkKbdKey | readonly SkKbdKey[] | HotkeyChord;
export type SkKbdSequence = { readonly sequence: readonly SkKbdChord[] };
export type SkKbdKeys = SkKbdChord | SkKbdSequence | { readonly alternatives: readonly (SkKbdChord | SkKbdSequence)[] };
export type SkKbdPlatform = "auto" | "windows" | "macos" | "linux";
export interface SkKbdLabel {
  readonly text: string;
  readonly label: string;
}
export interface SkKbdLabels {
  /** Canonical key names, or the displayed VK/Scan identity for a layout-specific captured key. */
  readonly keys?: Readonly<Record<string, string | SkKbdLabel>>;
  readonly plus?: string;
  readonly then?: string;
  readonly or?: string;
}
export interface SkKbdProps {
  keys?: SkKbdKeys | null | undefined;
  map?: SkKeyMap | null | undefined;
  platform?: SkKbdPlatform | undefined;
  /** Select legends from a skathi:gamepad controller. An explicit gamepadLayout overrides it. */
  controller?: ControllerIdentity | undefined;
  gamepadLayout?: SkKbdGamepadLayout | undefined;
  mousePrimary?: "left" | "right" | undefined;
  variant?: "keycaps" | "plain";
  labels?: SkKbdLabels | undefined;
}

function keyAliases() {
  const names = new Map<string, string>([
    ["mod", "Mod"],
    ["cmdorctrl", "Mod"],
    ["commandorcontrol", "Mod"],
    ["ctrl", "Control"],
    ["control", "Control"],
    ["⌃", "Control"],
    ["alt", "Alt"],
    ["option", "Alt"],
    ["opt", "Alt"],
    ["⌥", "Alt"],
    ["shift", "Shift"],
    ["⇧", "Shift"],
    ["meta", "Meta"],
    ["cmd", "Meta"],
    ["command", "Meta"],
    ["⌘", "Meta"],
    ["win", "Meta"],
    ["super", "Meta"],
    ["altgr", "AltGraph"],
    ["esc", "Escape"],
    ["return", "Enter"],
    ["del", "Delete"],
    ["up", "ArrowUp"],
    ["down", "ArrowDown"],
    ["left", "ArrowLeft"],
    ["right", "ArrowRight"],
    ["pgup", "PageUp"],
    ["pgdn", "PageDown"],
    ["plus", "+"],
    ["spacebar", "Space"],
    ["lmb", "Mouse1"],
    ["rmb", "Mouse2"],
    ["mmb", "Mouse3"],
    ["x1", "Mouse4"],
    ["x2", "Mouse5"],
    ...["Mouse1", "Mouse2", "Mouse3", "Mouse4", "Mouse5", "WheelUp", "WheelDown", "WheelLeft", "WheelRight"].map(
      (name): [string, string] => [name.toLowerCase(), name],
    ),
  ]);
  for (const name of Object.keys(common)) names.set(name.toLowerCase(), name);
  return names;
}

// A string supplies the spoken name too. Tuples hold the visible keycap and its spoken name.
type KeyName = string | readonly [string, string];
const common: Readonly<Record<string, KeyName>> = {
  Control: ["Ctrl", "Control"],
  Alt: "Alt",
  Shift: "Shift",
  AltGraph: ["AltGr", "Alt Graph"],
  Escape: ["Esc", "Escape"],
  Enter: "Enter",
  Tab: "Tab",
  Space: "Space",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: ["Ins", "Insert"],
  Home: "Home",
  End: "End",
  ArrowLeft: ["←", "Left arrow"],
  ArrowUp: ["↑", "Up arrow"],
  ArrowRight: ["→", "Right arrow"],
  ArrowDown: ["↓", "Down arrow"],
  PageUp: ["PgUp", "Page Up"],
  PageDown: ["PgDn", "Page Down"],
  CapsLock: "Caps Lock",
  NumLock: "Num Lock",
  ScrollLock: "Scroll Lock",
  ContextMenu: "Menu",
  PrintScreen: ["PrtSc", "Print Screen"],
  Fn: ["Fn", "Function"],
  "+": ["+", "Plus"],
};
const mac: Readonly<Record<string, KeyName>> = {
  Control: ["⌃", "Control"],
  Alt: ["⌥", "Option"],
  Shift: ["⇧", "Shift"],
  Meta: ["⌘", "Command"],
  Enter: ["↩", "Return"],
  Backspace: ["⌫", "Delete"],
  Delete: ["⌦", "Forward Delete"],
  Escape: ["⎋", "Escape"],
  Tab: ["⇥", "Tab"],
  CapsLock: ["⇪", "Caps Lock"],
};
const numpad: Readonly<Record<string, KeyName>> = {
  Enter: "Enter",
  Add: ["+", "Plus"],
  Subtract: ["−", "Minus"],
  Multiply: ["×", "Multiply"],
  Divide: ["/", "Divide"],
  Decimal: [".", "Decimal"],
  Separator: "Separator",
  Equal: ["=", "Equals"],
};
const virtualKeys: Readonly<Record<number, string>> = {
  0x08: "Backspace",
  0x09: "Tab",
  0x0c: "Clear",
  0x0d: "Enter",
  0x10: "Shift",
  0x11: "Control",
  0x12: "Alt",
  0x13: "Pause",
  0x14: "CapsLock",
  0x1b: "Escape",
  0x20: "Space",
  0x21: "PageUp",
  0x22: "PageDown",
  0x23: "End",
  0x24: "Home",
  0x25: "ArrowLeft",
  0x26: "ArrowUp",
  0x27: "ArrowRight",
  0x28: "ArrowDown",
  0x2c: "PrintScreen",
  0x2d: "Insert",
  0x2e: "Delete",
  0x5b: "MetaLeft",
  0x5c: "MetaRight",
  0x5d: "ContextMenu",
  0x6a: "NumpadMultiply",
  0x6b: "NumpadAdd",
  0x6c: "NumpadSeparator",
  0x6d: "NumpadSubtract",
  0x6e: "NumpadDecimal",
  0x6f: "NumpadDivide",
  0x90: "NumLock",
  0x91: "ScrollLock",
  0xa0: "ShiftLeft",
  0xa1: "ShiftRight",
  0xa2: "ControlLeft",
  0xa3: "ControlRight",
  0xa4: "AltLeft",
  0xa5: "AltRight",
  0xa6: "Browser Back",
  0xa7: "Browser Forward",
  0xad: "Mute",
  0xae: "Volume Down",
  0xaf: "Volume Up",
  0xb0: "Next Track",
  0xb1: "Previous Track",
  0xb2: "Stop",
  0xb3: "Play/Pause",
};
// sk-hotkeys distinguishes the keypad's navigation mode and Enter from the main keyboard.
const scanKeys: Readonly<Record<number, string>> = {
  0x0e1c: "NumpadEnter",
  0x007e: "NumpadSeparator",
  0xee47: "NumpadHome",
  0xee48: "NumpadArrowUp",
  0xee49: "NumpadPageUp",
  0xee4b: "NumpadArrowLeft",
  0xee4c: "NumpadClear",
  0xee4d: "NumpadArrowRight",
  0xee4f: "NumpadEnd",
  0xee50: "NumpadArrowDown",
  0xee51: "NumpadPageDown",
  0xee52: "NumpadInsert",
  0xee53: "NumpadDelete",
};

const aliases = /*@__PURE__*/ keyAliases();

export function buttonName({ device, code }: HotkeyButton): string {
  if (device === 1) return "Mouse" + code;
  const hex = (n: number) => "0x" + n.toString(16).toUpperCase().padStart(2, "0");
  if (device !== 0) return `Device ${device} · ${hex(code)}`;
  if (code & 0x10000) return scanKeys[code & 0xffff] ?? "Scan " + hex(code & 0xffff);
  if ((code >= 0x30 && code <= 0x39) || (code >= 0x41 && code <= 0x5a)) return String.fromCharCode(code);
  if (code >= 0x60 && code <= 0x69) return "Numpad" + (code - 0x60);
  if (code >= 0x70 && code <= 0x87) return "F" + (code - 0x6f);
  return virtualKeys[code] ?? "VK " + hex(code);
}

export function chordTokens(chord: SkKbdChord): readonly SkKbdKey[] {
  if (typeof chord !== "string") {
    if ("buttons" in chord) return chord.buttons.map(buttonName);
    if ("mouse" in chord || "gamepad" in chord || "code" in chord) return [chord];
    return chord;
  }
  if (chord === " ") return ["Space"];
  const source = chord.trim().replace(/\s*\+\s*/g, "+");
  if (source === "+") return ["+"];
  const plus = source.endsWith("++");
  const keys = (plus ? source.slice(0, -2) : source).split("+");
  if (keys.some((key) => !key)) return [];
  if (plus) keys.push("+");
  return keys;
}

export function canonical(token: SkKbdKey, apple: boolean): string {
  if (typeof token !== "string") return "code" in token ? token.code.trim() : deviceKeyId(token);
  if (token === " ") return "Space";
  const name = token.trim();
  const key = aliases.get(name.toLowerCase()) ?? name;
  if (key === "Mod") return apple ? "Meta" : "Control";
  return /^(?:[a-z]|f(?:[1-9]|1\d|2[0-4]))$/i.test(key) ? key.toUpperCase() : key;
}

function labelOf(value: KeyName): SkKbdLabel {
  return typeof value === "string" ? { text: value, label: value } : { text: value[0], label: value[1] };
}

function own<T>(map: Readonly<Record<string, T>> | undefined, key: string): T | undefined {
  return map && Object.hasOwn(map, key) ? map[key] : undefined;
}

function keyboardLabel(
  name: string,
  platform: SkKbdPlatform,
  labels: SkKbdLabels | undefined,
  position = name,
): SkKbdLabel {
  const override = own(labels?.keys, name);
  if (override !== undefined) return typeof override === "string" ? labelOf(override) : override;
  const legend = keyboardLegend(position);
  if (legend) return labelOf(legend);
  const standard = /^(?:Key([A-Z])|Digit([0-9]))$/.exec(name);
  if (standard) return labelOf(standard[1] ?? standard[2]!);
  const sided = /^(Control|Shift|Alt|Meta)(Left|Right)$/.exec(name);
  if (sided) {
    const key = keyboardLabel(sided[1]!, platform, labels);
    return { text: sided[2]![0] + " " + key.text, label: sided[2] + " " + key.label };
  }
  if (name.startsWith("Numpad") && name.length > 6) {
    const kind = name.slice(6);
    const key = labelOf(own(numpad, kind) ?? own(common, kind) ?? kind);
    return { text: "Num " + key.text, label: "Numpad " + key.label };
  }
  if (platform === "macos") {
    const key = own(mac, name);
    if (key) return labelOf(key);
  }
  if (name === "Meta") return labelOf(platform === "windows" ? ["Win", "Windows"] : "Super");
  return labelOf(own(common, name) ?? name);
}

export function platformOf(platform?: SkKbdPlatform): Exclude<SkKbdPlatform, "auto"> {
  if (platform && platform !== "auto") return platform;
  const os = globalThis.navigator?.platform ?? "";
  return /^(mac|darwin)/i.test(os) ? "macos" : /^win/i.test(os) ? "windows" : "linux";
}

/** @internal Shared render model for the Vue and Svelte components. */
export function kbdPresentation({
  keys,
  map,
  platform,
  labels,
  controller,
  gamepadLayout = controllerLayout(controller),
  mousePrimary = map?.mousePrimary ?? "left",
}: SkKbdProps) {
  keys = map ? map.keys : keys;
  const os = platformOf(platform);
  const apple = os === "macos";
  const modifiers = apple
    ? ["Fn", "Control", "Alt", "AltGraph", "Shift", "Meta"]
    : ["Meta", "Control", "Alt", "AltGraph", "Shift", "Fn"];
  function rank(key: string) {
    const index = modifiers.indexOf(key.replace(/(Left|Right)$/, ""));
    return index < 0 ? modifiers.length : index;
  }
  function keycap(name: string, position?: string): KbdKeycap {
    const device = deviceKeycap(name, gamepadLayout, mousePrimary, controller);
    if (!device) return keyboardLabel(name, os, labels, position);
    const override = own(labels?.keys, name);
    return override === undefined
      ? device
      : { ...device, ...(typeof override === "string" ? labelOf(override) : override) };
  }
  const groups: { before: string; keys: (KbdKeycap & { joiner: string })[]; text: string }[] = [];
  let spoken = "";
  const choices = keys == null ? [] : typeof keys === "object" && "alternatives" in keys ? keys.alternatives : [keys];
  for (const choice of choices) {
    const steps = typeof choice === "object" && "sequence" in choice ? choice.sequence : [choice];
    const chords = steps.map((step) => {
      const names = [...new Set(chordTokens(step).map((key) => canonical(key, apple)))];
      if (names.includes("")) return [];
      const captured = typeof step === "object" && "buttons" in step ? step.buttons : undefined;
      return names
        .sort((a, b) => rank(a) - rank(b))
        .map((name) => {
          const button = captured?.find((b) => buttonName(b) === name);
          return keycap(name, button && capturedPosition(button));
        });
    });
    if (!chords.length || chords.some((chord) => !chord.length)) continue;
    for (const [i, chord] of chords.entries()) {
      const before = i ? (labels?.then ?? "then") : groups.length ? (labels?.or ?? "or") : "";
      const caps = chord.map((key, index) => {
        const previous = chord[index - 1];
        const compact = previous && apple && !previous.device && /^[⌃⌥⇧⌘]$/.test(previous.text);
        return { ...key, joiner: previous && !compact ? "+" : "" };
      });
      groups.push({ before, keys: caps, text: caps.map((key) => key.joiner + key.text).join("") });
      if (before) spoken += ", " + before + " ";
      spoken += chord.map((key) => key.label).join(" " + (labels?.plus ?? "plus") + " ");
    }
  }
  return { groups, label: spoken };
}
