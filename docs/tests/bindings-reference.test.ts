import { expect, test } from "bun:test";
import { bindings } from "../scripts/bindings-reference";

test("reads each module overview from the dev loader registry", () => {
  const { moduleSummaries, std } = bindings();
  expect([...moduleSummaries.keys()].sort()).toEqual([...std.keys()].sort());
  expect(moduleSummaries.get("skathi:gamepad")).toContain("controller snapshots");
  expect(moduleSummaries.get("skathi:toml")).toContain('`import config from "./config.toml"`');
});

test("finds public types named by an API without pulling in their dependencies", () => {
  const { std, web, referencedTypes } = bindings();
  const gamepad = std.get("skathi:gamepad")!;
  expect(referencedTypes(gamepad, gamepad.get("gamepads")!)).toEqual(
    expect.arrayContaining(["Controller", "ControllerEvent", "DeadZoneTarget"]),
  );
  expect(referencedTypes(gamepad, gamepad.get("gamepads")!)).not.toContain("ControllerFamily");
  const related = referencedTypes(web, web.get("Gamepad")!);
  expect(related).toEqual(expect.arrayContaining(["GamepadButton", "GamepadHapticActuator"]));
  expect(related).not.toContain("GamepadEffectParameters");
});

test("Web declarations merge SDK interfaces, preserve overloads and exclude the host protocol", () => {
  const { web, syntax } = bindings();
  expect(web.has("__sk_set_prop")).toBe(false);
  expect(web.has("HTMLMarqueeElement")).toBe(false);
  expect(syntax(web, "Blob")).toContain("new Blob(");
  expect(syntax(web, "navigator.getGamepads")).toContain("getGamepads?");
  expect(syntax(web, "addEventListener")!.split("\n").length).toBeGreaterThan(1);
});

test("std modules include re-exports, type-only exports and the callable members of native objects", () => {
  const { std, syntax, describe, typeOnly } = bindings();
  const app = std.get("skathi:app")!;
  const gamepad = std.get("skathi:gamepad")!;
  expect(describe(app.get("appWindow")!).source).toBe("packages/runtime/src/runtime/window.ts");
  expect(typeOnly(gamepad.get("DeadZoneTarget")!)).toBe(true);
  expect(syntax(gamepad, "gamepads", true)).toContain("gamepads.setDeadZone(");
  expect(syntax(gamepad, "gamepads", true)).toContain("finite radius");
  const bitSet = syntax(std.get("skathi:collections")!, "BitSet", true);
  expect(bitSet).toContain("BitSet extends Iterable<number>");
  expect(bitSet).toContain("set(index: number): this;");
  expect(bitSet).not.toContain("BitSet.prototype");
  const timer = syntax(std.get("skathi:time")!, "Timer", true);
  expect(timer).toContain("class Timer extends EventTarget {");
  expect(timer).toContain("readonly remaining: number;");
  expect(timer).not.toContain("dispatchEvent(");
  expect(syntax(std.get("skathi:binary")!, "BinaryReader", true)).toContain("type NumberArray =");
  expect(syntax(std.get("skathi:network")!, "network", true)).toContain("offline: Event;");
  expect(describe(gamepad.get("Gamepads")!, true).syntax).toContain("finite radius");
  const target = describe(gamepad.get("DeadZoneTarget")!, true);
  expect(target.summary).toContain("A model uses its USB");
  expect(target.syntax).not.toContain("A model uses its USB");
  expect(describe(gamepad.get("gamepads")!, true).syntax).toContain("finite radius");
  expect(describe(std.get("skathi:numeric")!.get("i32")!, true).syntax).toContain("i32.add(");
  const wgslType = describe(std.get("skathi:gpu")!.get("WgslType")!, true).syntax;
  expect(wgslType).not.toContain("new WgslType");
  expect(wgslType).not.toContain("WgslType.node");
  expect(wgslType).not.toContain("#node");
});
