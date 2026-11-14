import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

interface RangeProps {
  min: string;
  max: string;
  step: string;
  defaultValue: string;
  valueAsNumber: number;
  stepUp(n?: unknown): void;
  stepDown(n?: unknown): void;
}

function world(attrs: Record<string, string> = {}) {
  const rt = createRuntime();
  const root = rt.nodeOps.createElement("root");
  const input = rt.nodeOps.createElement("input") as MirrorElement & RangeProps;
  rt.nodeOps.insert(input, root, null);
  rt.patchProp(input, "type", null, "range");
  for (const [key, value] of Object.entries(attrs)) rt.patchProp(input, key, null, value);
  const seen: string[] = [];
  for (const name of ["input", "change"])
    rt.patchProp(
      root,
      "on" + name[0]!.toUpperCase() + name.slice(1),
      null,
      (e: { target: { value: string }; bubbles: boolean }) => seen.push(name + ":" + e.target.value + ":" + e.bubbles),
    );
  return {
    rt,
    input,
    seen,
    host: (value: string, commit = false) =>
      rt.host.dispatcher(input.__skId, rt.EV_CODES.input, 0, commit ? 1 : 0, 0, 0, value),
  };
}

describe("HTML input range", () => {
  test("default and invalid values use midpoint; bounds and positive step ties sanitize", () => {
    const { input } = world();
    expect(input.value).toBe("50");
    for (const value of ["", "NaN", "infinity", " 25", "1."]) {
      input.value = value;
      expect(input.value).toBe("50");
    }
    input.value = "0.5";
    expect(input.value).toBe("1");
    input.value = "-500";
    expect(input.value).toBe("0");
    input.value = "500";
    expect(input.value).toBe("100");
    input.min = "200";
    input.max = "100";
    expect(input.value).toBe("200");
  });
  test("the actual nickname gauges and fractional steps retain their range", () => {
    const { input } = world({ min: "100", max: "900", step: "100", value: "900" });
    expect(input.valueAsNumber).toBe(900);
    input.value = "849";
    expect(input.value).toBe("800");
    input.stepDown();
    expect(input.value).toBe("700");
    input.stepUp(2);
    expect(input.value).toBe("900");
    input.stepDown(null);
    expect(input.value).toBe("900");
    input.min = "-10";
    input.max = "50";
    input.step = "1";
    input.value = "26";
    expect(input.value).toBe("26");
    input.stepUp();
    expect(input.value).toBe("27");
    input.min = "0";
    input.max = "1";
    input.step = "0.1";
    input.valueAsNumber = 0.3;
    expect(input.value).toBe("0.3");
    input.valueAsNumber = NaN;
    expect(input.value).toBe("0.5");
    input.step = "any";
    input.value = "0.333";
    expect(input.value).toBe("0.333");
    expect(() => input.stepUp()).toThrow();
  });
  test("native edits bubble input during drag, then exactly one committed change", () => {
    const { input, seen, host } = world({ min: "100", max: "900", step: "100", value: "100" });
    host("200");
    host("300");
    host("300", true);
    expect(seen).toEqual(["input:200:true", "input:300:true", "change:300:true"]);
    expect(input.valueAsNumber).toBe(300);
    (input as { value: unknown }).value = 500;
    input.stepUp();
    expect(seen).toHaveLength(3);
  });
  test("attribute step base and Vue v-model receive the same normalized value", () => {
    const { rt, input, seen, host } = world({ max: "10", step: "1" });
    input.setAttribute("value", "0.2");
    input.value = "1.6";
    expect(input.value).toBe("1.2");
    let model: unknown;
    rt.vModelText.created(
      input,
      { modifiers: {} },
      {
        props: {
          "onUpdate:modelValue": (v: unknown) => {
            model = v;
          },
        },
      },
    );
    host("2.2");
    expect(model).toBe("2.2");
    expect(seen).toEqual(["input:2.2:true"]);
  });
  test("video volume/rate decimals and v-model.number stay numeric", () => {
    const { rt, input, host, seen } = world({ min: "0", max: "1", step: "0.01", value: "0.75" });
    input.stepDown();
    expect(input.value).toBe("0.74");
    host("0.42");
    host("0.42", true);
    expect(seen).toEqual(["input:0.42:true", "change:0.42:true"]);
    input.min = "0.25";
    input.max = "4";
    input.step = "0.05";
    input.value = "1.1";
    input.stepUp();
    expect(input.value).toBe("1.15");
    let model: unknown;
    rt.vModelText.created(
      input,
      { modifiers: { number: true } },
      {
        props: {
          "onUpdate:modelValue": (v: unknown) => {
            model = v;
          },
        },
      },
    );
    host("1.2");
    expect(model).toBe(1.2);
    expect(typeof model).toBe("number");
  });
  test("DOM assignment order follows Chromium and defaults preserve dirty user values", () => {
    const { rt, input, host } = world();
    expect(input.value).toBe("50");
    input.min = "100";
    expect(input.value).toBe("100");
    input.max = "900";
    expect(input.value).toBe("100");
    input.defaultValue = "300";
    expect(input.value).toBe("300");
    input.value = "500";
    input.defaultValue = "700";
    expect(input.value).toBe("500");
    expect(input.defaultValue).toBe("700");
    host("600");
    input.defaultValue = "800";
    expect(input.value).toBe("600");
    const form = rt.nodeOps.createElement("form");
    rt.nodeOps.insert(input, form, null);
    (form as MirrorElement & { reset(): void }).reset();
    expect(input.value).toBe("800");
    input.defaultValue = "200";
    expect(input.value).toBe("200");
    // Vue's :value is a live property write, so it still controls the range.
    rt.patchProp(input, "value", null, 400);
    expect(input.value).toBe("400");
    input.defaultValue = "300";
    expect(input.value).toBe("400");
  });
  test("readonly does not remove range operation and change is supported", () => {
    const { rt, input } = world({ readonly: "", min: "0", max: "10", value: "4" });
    input.stepUp();
    expect(input.value).toBe("5");
    expect(rt.advisories().filter((s: string) => s.includes("@change"))).toEqual([]);
  });
});
