import { describe, expect, test } from "bun:test";
import {
  kbdPresentation,
  type SkKbdGamepadLayout,
  type SkKbdKeys,
  type SkKbdPlatform,
  type SkKbdProps,
} from "../src/kbd";

const format = (keys: SkKbdKeys | null, platform: SkKbdPlatform = "windows") => kbdPresentation({ keys, platform });
const text = (keys: SkKbdKeys, platform?: SkKbdPlatform) => format(keys, platform).groups.map((group) => group.text);

describe("shortcut labels", () => {
  test("resolves the primary modifier and orders shortcuts per platform", () => {
    expect(text("p+Shift+Mod", "macos")).toEqual(["⇧⌘P"]);
    expect(text("p+Shift+Mod", "windows")).toEqual(["Ctrl+Shift+P"]);
    expect(text("Shift+Meta+s", "linux")).toEqual(["Super+Shift+S"]);
    expect(text("Ctrl+Mod+k", "macos")).toEqual(["⌃⌘K"]);
    expect(text("Ctrl+Mod+k")).toEqual(["Ctrl+K"]);
    expect(text("a+b", "macos")).toEqual(["A+B"]);
    expect(text("Fn+KeyE", "macos")).toEqual(["Fn+E"]);
    expect(text("Meta+Digit7", "macos")).toEqual(["⌘7"]);
    expect(format("Option+Escape", "macos").label).toBe("Option plus Escape");
  });

  test("preserves printable keys, space, simultaneous non-modifiers and a literal plus", () => {
    expect(text("Ctrl + +")).toEqual(["Ctrl++"]);
    expect(text(["mod", "+"])).toEqual(text("Mod+Plus"));
    expect(format("+").label).toBe("Plus");
    expect(text(["Ж", "ß", ","])).toEqual(["Ж+ß+,"]);
    expect(text(" ")).toEqual(["Space"]);
    expect(text("F24+f1")).toEqual(["F24+F1"]);
    expect(text("constructor")).toEqual(["constructor"]);
  });

  test("keeps order within a sequence and alternatives between whole sequences", () => {
    const model = format({ alternatives: [{ sequence: ["g", "g"] }, "Mod+Home"] });
    expect(model.groups.map(({ before, text }) => [before, text])).toEqual([
      ["", "G"],
      ["then", "G"],
      ["or", "Ctrl+Home"],
    ]);
    expect(model.label).toBe("G, then G, or Control plus Home");
    expect(text({ alternatives: [{ sequence: ["Mod+K", ""] }, "F1"] })).toEqual(["F1"]);
    for (const missing of [null, "", "Ctrl+", [], { buttons: [] }, { sequence: [] }, { alternatives: [] }] as const) {
      expect(format(missing).groups).toEqual([]);
    }
  });

  test("keeps recorded modifier sides, keypad identity, unknown scans and mouse devices", () => {
    const model = format({
      buttons: [
        { device: 1, code: 4 },
        { device: 0, code: 0xa3 },
        { device: 0, code: 0x10e1c },
        { device: 0, code: 0x0d },
        { device: 0, code: 0x1ee52 },
        { device: 0, code: 0x61 },
        { device: 0, code: 0x1e012 },
        { device: 0, code: 0xba },
        { device: 2, code: 9 },
      ],
    });
    expect(model.groups[0]!.keys.map((key) => key.text)).toEqual([
      "R Ctrl",
      "Mouse 4",
      "Num Enter",
      "Enter",
      "Num Ins",
      "Num 1",
      "Scan 0xE012",
      "VK 0xBA",
      "Device 2 · 0x09",
    ]);
    expect(format({ buttons: [{ device: 0, code: 0x5c }] }, "macos").label).toBe("Right Command");
  });

  test("accepts key legends and spoken names from the app's locale or keyboard layout", () => {
    const model = kbdPresentation({
      keys: { sequence: [{ buttons: [{ device: 0, code: 0xba }] }, ["Control", "Enter"]] },
      platform: "windows",
      labels: { plus: "и", then: "затем", keys: { "VK 0xBA": "Ж", Enter: { text: "↵", label: "Ввод" } } },
    });
    expect(model.label).toBe("Ж, затем Control и Ввод");
    expect(model.groups[1]!.text).toBe("Ctrl+↵");
  });
});

describe("mouse and controller prompts", () => {
  const cap = (keys: SkKbdKeys, props: Omit<SkKbdProps, "keys"> = {}) =>
    kbdPresentation({ keys, ...props }).groups[0]?.keys[0];

  test("DOM button indices, physical hotkey codes and scroll directions keep their identities", () => {
    const dom = [0, 1, 2, 3, 4].map((mouse) => cap({ mouse })!);
    expect(dom.map((key) => key.label)).toEqual([
      "Primary mouse button",
      "Middle mouse button",
      "Secondary mouse button",
      "Back mouse button",
      "Forward mouse button",
    ]);
    expect(new Set(dom.map((key) => key.glyph!.fill)).size).toBe(5);
    const recorded = (code: number) => cap({ buttons: [{ device: 1, code }] })!;
    expect(dom[0]!.glyph).toEqual(recorded(1).glyph);
    expect(dom[1]!.glyph).toEqual(recorded(3).glyph);
    expect(dom[2]!.glyph).toEqual(recorded(2).glyph);
    expect(dom[3]!.glyph).toEqual(recorded(4).glyph);
    expect(dom[4]!.glyph).toEqual(recorded(5).glyph);
    expect(cap({ mouse: 0 }, { mousePrimary: "right" })?.glyph).toEqual(recorded(2).glyph);
    expect(cap({ mouse: "left" }, { mousePrimary: "right" })?.glyph).toEqual(recorded(1).glyph);
    expect(cap("RMB")).toEqual(recorded(2));
    expect(cap({ mouse: 5 })?.text).toBe("Mouse 6");
    expect(cap({ mouse: -1 })).toBeUndefined();
    expect(cap({ mouse: NaN })).toBeUndefined();
    const directions = ["wheel-up", "wheel-down", "wheel-left", "wheel-right"] as const;
    expect(new Set(directions.map((mouse) => cap({ mouse })?.glyph?.outline)).size).toBe(4);
    expect(format(["Mod", { mouse: "wheel-up" }]).label).toBe("Control plus Scroll up");
  });

  test("the 17 standard button indices address physical positions, independent of controller lettering", () => {
    const positions = [
      "south",
      "east",
      "west",
      "north",
      "left-bumper",
      "right-bumper",
      "left-trigger",
      "right-trigger",
      "select",
      "start",
      "left-stick",
      "right-stick",
      "dpad-up",
      "dpad-down",
      "dpad-left",
      "dpad-right",
      "home",
    ] as const;
    const families = {
      generic: ["South", "East", "West", "North"],
      xbox: ["A", "B", "X", "Y"],
      playstation: ["×", "○", "□", "△"],
      nintendo: ["B", "A", "Y", "X"],
    } as const;
    for (const layout of Object.keys(families) as SkKbdGamepadLayout[]) {
      const props = { gamepadLayout: layout };
      const caps = positions.map((gamepad, index) => {
        const authored = cap({ gamepad }, props);
        expect(authored).toEqual(cap({ gamepad: index, mapping: "standard" }, props));
        return authored!;
      });
      expect(caps.slice(0, 4).map((key) => key.text)).toEqual([...families[layout]]);
      expect(new Set(caps.slice(0, 4).map((key) => key.glyph!.fill)).size).toBe(4);
      expect(new Set(caps.slice(12, 16).map((key) => key.glyph!.fill)).size).toBe(4);
    }
    expect(cap({ gamepad: "west" }, { gamepadLayout: "playstation" })?.label).toBe("Square button");
    expect(cap({ gamepad: "start" }, { gamepadLayout: "nintendo" })?.label).toBe("Plus button");
    expect(cap({ gamepad: "left-stick" }, { gamepadLayout: "playstation" })?.label).toBe("L3, left stick press");
  });

  test("keycaps and plain text keep the chord between two controller buttons on macOS", () => {
    const { keys, text } = kbdPresentation({
      keys: ["Mod", { gamepad: "left-trigger" }, { gamepad: "right-trigger" }],
      platform: "macos",
      gamepadLayout: "playstation",
    }).groups[0]!;
    expect(keys.map((key) => key.joiner)).toEqual(["", "", "+"]);
    expect(text).toBe("⌘L2+R2");
  });

  test("unmapped and extra buttons keep their raw index and accept the device author's legend", () => {
    for (const mapping of ["", "xr-standard", "custom"])
      expect(cap({ gamepad: 0, mapping }, { gamepadLayout: "xbox" })?.text).toBe("Pad 0");
    expect(cap({ gamepad: 0 })?.text).toBe("Pad 0");
    expect(cap({ gamepad: 17, mapping: "standard" })?.label).toBe("Gamepad button 17");
    expect(cap({ gamepad: 0.5 })).toBeUndefined();
    expect(cap({ gamepad: -1, mapping: "standard" })).toBeUndefined();
    const custom = {
      keys: { GamepadButton17: { text: "Touchpad", label: "Press touchpad" }, "Gamepad:south": "Confirm" },
    };
    expect(cap({ gamepad: 17 }, { labels: custom })?.label).toBe("Press touchpad");
    expect(cap({ gamepad: "south" }, { labels: custom })).toMatchObject({
      text: "Confirm",
      shape: "face",
      device: "gamepad",
    });
    expect(
      kbdPresentation({
        keys: {
          alternatives: [["Shift", { mouse: "back" }], { sequence: [{ gamepad: "south" }, { gamepad: "east" }] }],
        },
        gamepadLayout: "xbox",
      }).label,
    ).toBe("Shift plus Back mouse button, or A button, then B button");
  });
});
