import { expect, test } from "bun:test";
import {
  assign,
  controlsFor,
  cssText,
  draftFrom,
  parseShading,
  readEffects,
  type EffectInfo,
} from "../src/effects-model";

const param = (name: string, kind: string, slot: number, required: boolean, def: number[], range: number[] | null) => ({
  name,
  kind,
  slot,
  required,
  default: def,
  range,
});

const manifest = {
  schema: 99,
  kind: "builtin",
  effects: [
    {
      name: "noise",
      doc: "film grain",
      backdrop: false,
      signature: "noise(<number:amount>, <length:scale>?, <number:speed>?)",
      bleedParam: null,
      animatedParam: 2,
      params: [
        param("amount", "number", 0, true, [0.25, 0, 0, 0], [0, 1]),
        param("scale", "length", 1, false, [1.5, 0, 0, 0], [0.05, 64]),
        param("speed", "number", 2, false, [0, 0, 0, 0], [0, 120]),
      ],
    },
    {
      name: "glow",
      backdrop: false,
      signature: "glow(<color>?, <length:radius>, <number:strength>)",
      bleedParam: 0,
      animatedParam: null,
      params: [
        param("color", "color", 4, false, [1, 1, 1, 1], null),
        param("radius", "length", 0, true, [16, 0, 0, 0], [0, 256]),
        param("strength", "number", 1, true, [0.75, 0, 0, 0], [0, 1]),
      ],
    },
  ],
};

const app = {
  schema: 99,
  kind: "user",
  effects: [
    {
      name: "lens",
      backdrop: true,
      backdropRadiusParam: 3,
      signature: "lens(<number:center.x>, <number:center.y>, <length:size>?)",
      futureMarker: true,
      params: [
        param("center.x", "number", 0, true, [0.5, 0, 0, 0], null),
        param("center.y", "number", 1, true, [0.5, 0, 0, 0], null),
        param("size", "length", 2, false, [40, 0, 0, 0], null),
        { ...param("pointer", "pointer", 12, false, [0, 0], null), host: "pointer" },
      ],
    },
  ],
};

function effects(): EffectInfo[] {
  return readEffects(
    [
      { origin: "builtin", builtin: true, manifest },
      { origin: "the app's payload", builtin: false, manifest: app },
    ],
    [
      { name: "noise", id: 0, builtin: true },
      { name: "glow", id: 1, builtin: true },
      { name: "lens", id: 2, builtin: false },
    ],
  );
}

const named = (name: string): EffectInfo => effects().find((e) => e.name === name)!;

test("a manifest with fields and kinds this panel does not know still reads", () => {
  const lens = named("lens");
  expect(lens.loaded).toBe(true);
  expect(lens.params.map((p) => [p.name, p.editable])).toEqual([
    ["center.x", true],
    ["center.y", true],
    ["size", true],
    ["pointer", false],
  ]);
  expect(lens.flags.map((f) => f.key)).toEqual(["backdrop", "futureMarker"]);
  expect(named("noise").flags.map((f) => f.key)).toEqual(["animated"]);
  expect(named("glow").flags.map((f) => f.key)).toEqual(["bleed"]);
});

test("vector components share a control and a host-filled parameter gets no editor", () => {
  const controls = controlsFor(named("lens"));
  expect(controls.map((c) => c.type)).toEqual(["vector", "scalar", "fixed"]);
  const vector = controls[0]!;
  expect(vector.type === "vector" && vector.parts.map((p) => p.axis)).toEqual(["x", "y"]);
  const size = controls[1]!;
  expect(size.type === "scalar" && size.scalar.unit).toBe("px");
  expect(size.type === "scalar" && size.scalar.max).toBeGreaterThanOrEqual(40);
});

test("arguments land on parameters the way the host matches them", () => {
  const glow = named("glow");
  expect([...assign(glow, parseShading("glow(12px, 0.8)")!.args)]).toEqual([
    ["radius", "12px"],
    ["strength", "0.8"],
  ]);
  expect([...assign(glow, parseShading("glow(rgb(255, 0, 0), 12px, 0.8)")!.args).keys()]).toEqual([
    "color",
    "radius",
    "strength",
  ]);
});

test("a draft is read from the declaration and written back unchanged", () => {
  const glow = named("glow");
  const draft = draftFrom(glow, "glow(#ff000080, 12px, 0.8)", null);
  expect(draft.enabled).toEqual({ color: true, radius: true, strength: true });
  expect(cssText(glow, draft)).toBe("glow(#ff000080, 12px, 0.8)");

  const resolved = [20, 0.5, 0, 0, 1, 1, 1, 1];
  const fromHost = draftFrom(glow, "glow(20px, 0.5)", resolved);
  expect(fromHost.enabled.color).toBe(false);
  expect(cssText(glow, fromHost)).toBe("glow(20px, 0.5)");
});

test("a switched-off optional length is still written when a following 0 would land on it", () => {
  const noise = named("noise");
  const draft = draftFrom(noise, "noise(0.3)", null);
  draft.enabled.speed = true;
  draft.values.speed = [0];
  const text = cssText(noise, draft);
  expect(text).toBe("noise(0.3, 1.5px, 0)");
  expect(assign(noise, parseShading(text)!.args).get("speed")).toBe("0");
});
