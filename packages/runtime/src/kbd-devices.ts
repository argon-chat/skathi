import type { SkKbdLabel } from "./kbd";
import type { Controller } from "./std/gamepad";

// Row indices follow the W3C standard mapping.
const gamepadButtons = [
  ["south", "South", "South face button", "A", "×", "B"],
  ["east", "East", "East face button", "B", "○", "A"],
  ["west", "West", "West face button", "X", "□", "Y"],
  ["north", "North", "North face button", "Y", "△", "X"],
  ["left-bumper", "L bumper", "Left bumper", "LB", "L1", "L"],
  ["right-bumper", "R bumper", "Right bumper", "RB", "R1", "R"],
  ["left-trigger", "L trigger", "Left trigger", "LT", "L2", "ZL"],
  ["right-trigger", "R trigger", "Right trigger", "RT", "R2", "ZR"],
  ["select", "Center L", "Left center button", "View", "Create", "−"],
  ["start", "Center R", "Right center button", "Menu", "Options", "+"],
  ["left-stick", "L stick", "Left stick press", "LS", "L3", "L stick"],
  ["right-stick", "R stick", "Right stick press", "RS", "R3", "R stick"],
  ["dpad-up", "D-pad ↑", "D-pad up"],
  ["dpad-down", "D-pad ↓", "D-pad down"],
  ["dpad-left", "D-pad ←", "D-pad left"],
  ["dpad-right", "D-pad →", "D-pad right"],
  ["home", "Home", "Home button", "Xbox", "PS", "Home"],
] as const;
export type SkKbdGamepadButton = (typeof gamepadButtons)[number][0];
export type SkKbdGamepadLayout = "generic" | "xbox" | "playstation" | "dualshock4" | "nintendo" | "steam";

export type ControllerIdentity = Pick<Controller, "family" | "vendorId" | "productId" | "name"> &
  Partial<Pick<Controller, "mapping">>;

export function controllerLayout(pad?: ControllerIdentity): SkKbdGamepadLayout {
  if (!pad) return "generic";
  if (
    pad.family === "playstation" &&
    ((pad.vendorId === 0x054c && [0x05c4, 0x09cc, 0x0ba0].includes(pad.productId)) || /dualshock\s*4/i.test(pad.name))
  ) {
    return "dualshock4";
  }
  return pad.family;
}

const mouseNames = {
  primary: "MousePrimary",
  secondary: "MouseSecondary",
  left: "Mouse1",
  right: "Mouse2",
  middle: "Mouse3",
  back: "MouseBack",
  forward: "MouseForward",
  "wheel-up": "WheelUp",
  "wheel-down": "WheelDown",
  "wheel-left": "WheelLeft",
  "wheel-right": "WheelRight",
} as const;
export type SkKbdMouseButton = keyof typeof mouseNames;
export type SkKbdDeviceInput =
  | { readonly mouse: SkKbdMouseButton | number }
  | { readonly gamepad: SkKbdGamepadButton }
  | { readonly gamepad: number; readonly mapping?: string };

type Glyph = { outline: string; fill?: string };
export interface KbdKeycap extends SkKbdLabel {
  device?: "mouse" | "gamepad";
  shape?: "face" | "shoulder" | "trigger" | "stick";
  glyph?: Glyph;
}

const domButtons = ["MousePrimary", "Mouse3", "MouseSecondary", "MouseBack", "MouseForward"] as const;

export function deviceKeyId(input: SkKbdDeviceInput): string {
  if ("mouse" in input) {
    const button = input.mouse;
    if (typeof button === "string") return Object.hasOwn(mouseNames, button) ? mouseNames[button] : "";
    if (!Number.isSafeInteger(button) || button < 0) return "";
    return domButtons[button] ?? "Mouse" + (button + 1);
  }
  const button = input.gamepad;
  if (typeof button === "string") return gamepadButtons.some(([id]) => id === button) ? "Gamepad:" + button : "";
  if (!Number.isSafeInteger(button) || button < 0) return "";
  const standard = "mapping" in input && input.mapping === "standard" ? gamepadButtons[button] : undefined;
  return standard ? "Gamepad:" + standard[0] : "GamepadButton" + button;
}

const mouseBody =
  "M11.5 2.5a6.5 6.5 0 0 1 6.5 6.5v6a6.5 6.5 0 0 1-13 0V9a6.5 6.5 0 0 1 6.5-6.5z M5 11.5h13 M11.5 2.5v9";
const wheel = "M11.5 5a1.5 1.5 0 0 1 1.5 1.5v2a1.5 1.5 0 0 1-3 0v-2A1.5 1.5 0 0 1 11.5 5z";
const mouseShapes = {
  left: "M10.6 3.5v7.1H5.9V9a5.6 5.6 0 0 1 4.7-5.5z",
  right: "M12.4 3.5a5.6 5.6 0 0 1 4.7 5.5v1.6h-4.7z",
  middle: wheel,
  back: "M1.4 13.1h3v4.5h-3z",
  forward: "M1.4 7.4h3v4.5h-3z",
} as const;
const wheelArrows = {
  WheelUp: "M21 12V4m-2.5 2.5L21 4l2.5 2.5",
  WheelDown: "M21 4v8m-2.5-2.5L21 12l2.5-2.5",
  WheelLeft: "M16 17H7m3-3-3 3 3 3",
  WheelRight: "M7 17h9m-3-3 3 3-3 3",
} as const;

type MousePart = keyof typeof mouseShapes | keyof typeof wheelArrows;
const mouseCaps: Readonly<Record<string, readonly [string, string, MousePart]>> = {
  Mouse1: ["LMB", "Left mouse button", "left"],
  Mouse2: ["RMB", "Right mouse button", "right"],
  Mouse3: ["MMB", "Middle mouse button", "middle"],
  Mouse4: ["Mouse 4", "Mouse button 4", "back"],
  Mouse5: ["Mouse 5", "Mouse button 5", "forward"],
  MouseBack: ["Back", "Back mouse button", "back"],
  MouseForward: ["Forward", "Forward mouse button", "forward"],
  WheelUp: ["Wheel ↑", "Scroll up", "WheelUp"],
  WheelDown: ["Wheel ↓", "Scroll down", "WheelDown"],
  WheelLeft: ["Wheel ←", "Scroll left", "WheelLeft"],
  WheelRight: ["Wheel →", "Scroll right", "WheelRight"],
};

function mouseKey(name: string, primary: "left" | "right"): KbdKeycap | undefined {
  let cap = Object.hasOwn(mouseCaps, name) ? mouseCaps[name] : undefined;
  if (name === "MousePrimary") cap = ["Primary", "Primary mouse button", primary];
  if (name === "MouseSecondary") cap = ["Secondary", "Secondary mouse button", primary === "left" ? "right" : "left"];
  if (!cap) {
    const extra = /^Mouse(\d+)$/.exec(name);
    return extra ? { device: "mouse", text: "Mouse " + extra[1], label: "Mouse button " + extra[1] } : undefined;
  }
  const [text, label, part] = cap;
  const glyph =
    part in wheelArrows
      ? { outline: mouseBody + " " + wheel + " " + wheelArrows[part as keyof typeof wheelArrows] }
      : { outline: mouseBody + " " + wheel, fill: mouseShapes[part as keyof typeof mouseShapes] };
  return { text, label, device: "mouse", glyph };
}

const legendColumn = { generic: 1, xbox: 3, playstation: 4, dualshock4: 4, nintendo: 5, steam: 3 } as const;
const steamLegends: Readonly<Record<string, string>> = { select: "Back", start: "Start", home: "Steam" };
// The Steam Controller Puck transport appends these controls after the 17 standard buttons.
const steamPuckButtons = [
  ["Quick Access", "Quick Access button"],
  ["R4", "R4, right grip button"],
  ["L4", "L4, left grip button"],
  ["R5", "R5, right grip button"],
  ["L5", "L5, left grip button"],
  ["Right Pad Click", "Right trackpad click"],
  ["Left Pad Click", "Left trackpad click"],
  ["Left Trigger Click", "Left trigger click"],
  ["Right Trigger Click", "Right trigger click"],
] as const;
const playstationShapes = ["Cross", "Circle", "Square", "Triangle"] as const;
const diamond = ["M12 15.5", "M18 9.5", "M6 9.5", "M12 3.5"].map(
  (point) => point + "a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5z",
);
const faceOutline = diamond.join(" ");
const dpadOutline = "M9 2.5h6V9h6.5v6H15v6.5H9V15H2.5V9H9z";
const dpadFill = ["M9 2.5h6V10H9z", "M9 14h6v7.5H9z", "M2.5 9H10v6H2.5z", "M14 9h7.5v6H14z"];

function gamepadKey(position: string, layout: SkKbdGamepadLayout): KbdKeycap | undefined {
  const i = gamepadButtons.findIndex(([id]) => id === position);
  const row = gamepadButtons[i];
  if (!row) return undefined;
  const generic = row[1],
    name = row[2];
  const legend =
    layout === "steam"
      ? steamLegends[position]
      : layout === "dualshock4" && position === "select"
        ? "Share"
        : undefined;
  const text = legend ?? row[legendColumn[layout]] ?? generic;
  const device = "gamepad";
  if (i < 4) {
    const printed = layout === "playstation" || layout === "dualshock4" ? playstationShapes[i] : text;
    return {
      device,
      text,
      label: layout === "generic" ? name : printed + " button",
      shape: "face",
      glyph: { outline: faceOutline, fill: diamond[i]! },
    };
  }
  if (i >= 12 && i <= 15)
    return { device, text, label: name, glyph: { outline: dpadOutline, fill: dpadFill[i - 12]! } };
  if (i === 8 || i === 9 || i === 16) {
    const printed = text === "−" ? "Minus" : text === "+" ? "Plus" : text;
    return { device, text, label: layout === "generic" ? name : printed + " button" };
  }
  const shape = i < 6 ? "shoulder" : i < 8 ? "trigger" : "stick";
  return { device, text, shape, label: text === generic ? name : text + ", " + name.toLowerCase() };
}

export function deviceKeycap(
  name: string,
  layout: SkKbdGamepadLayout,
  primary: "left" | "right",
  controller?: ControllerIdentity,
): KbdKeycap | undefined {
  if (name.startsWith("Mouse") || name.startsWith("Wheel")) return mouseKey(name, primary);
  if (name.startsWith("Gamepad:")) return gamepadKey(name.slice(8), layout);
  const raw = /^GamepadButton(\d+)$/.exec(name);
  if (!raw) return undefined;
  if (
    layout === "steam" &&
    controller?.family === "steam" &&
    controller.vendorId === 0x28de &&
    controller.productId === 0x1304 &&
    controller.mapping === "standard"
  ) {
    const legend = steamPuckButtons[Number(raw[1]) - gamepadButtons.length];
    if (legend) return { text: legend[0], label: legend[1], device: "gamepad" };
  }
  return { text: "Pad " + raw[1], label: "Gamepad button " + raw[1], device: "gamepad" };
}
