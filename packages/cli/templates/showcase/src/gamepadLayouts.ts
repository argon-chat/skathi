import type { SkKbdGamepadLayout } from "@skathi-native/runtime";

export const gamepadLayouts = [
  ["auto", "Connected controller"],
  ["generic", "Generic"],
  ["xbox", "Xbox"],
  ["playstation", "PlayStation · DualSense"],
  ["dualshock4", "PlayStation · DualShock 4"],
  ["steam", "Steam"],
  ["nintendo", "Nintendo · Switch Pro"],
] as const satisfies ReadonlyArray<readonly [SkKbdGamepadLayout | "auto", string]>;
