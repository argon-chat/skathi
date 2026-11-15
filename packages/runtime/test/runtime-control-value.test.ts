// Form-control `value` / `checked` as real accessors: reka-ui's VisuallyHiddenInputBubble and BubbleSelect set a
// control's value through the prototype's descriptor and dispatch `input` / `change` themselves, which needs a
// `value` member on the tag classes.

import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

type El = {
  __skId: number;
  value: unknown;
  checked: unknown;
  defaultValue: unknown;
  defaultChecked: unknown;
  attrs: Record<string, string>;
  addEventListener(t: string, f: (e: Event & { target: El }) => void): void;
  dispatchEvent(e: Event): boolean;
};
type Rt = ReturnType<typeof createRuntime>;

const lastValue = (rt: Rt, id: number, key = "value") =>
  rt.host.props.filter((p: { id: number; key: string }) => p.id === id && p.key === key).at(-1)?.value;
const proto = (rt: Rt, name: string): object => {
  const ctor = (rt.global as Record<string, { prototype: object } | undefined>)[name];
  if (!ctor) throw new Error(`${name} is not a realm global`);
  return ctor.prototype;
};
const mirror = (el: El) => el as unknown as MirrorElement;

describe("the reka bubble-input pattern", () => {
  test("HTMLInputElement.prototype's value setter reaches the host, and the hand-made events are heard", () => {
    const rt = createRuntime();
    const root = rt.nodeOps.createElement("form") as unknown as El;
    const el = rt.nodeOps.createElement("input") as unknown as El;
    rt.nodeOps.insert(mirror(el), mirror(root), null);
    const seen: string[] = [];
    el.addEventListener("input", (e) => seen.push(`input:${e.target.value}:${e.bubbles}`));
    el.addEventListener("change", (e) => seen.push(`change:${e.target.value}`));
    root.addEventListener("change", () => seen.push("change@form"));

    const cur: unknown = true;
    const prev: unknown = undefined;
    const input = el;
    const inputProto = proto(rt, "HTMLInputElement");
    const descriptor = Object.getOwnPropertyDescriptor(inputProto, "value")!;
    const setValue = descriptor.set;
    if (setValue && cur !== prev) {
      const inputEvent = new Event("input", { bubbles: true });
      const changeEvent = new Event("change", { bubbles: true });
      setValue.call(input, cur);
      input.dispatchEvent(inputEvent);
      input.dispatchEvent(changeEvent);
    }

    expect(lastValue(rt, el.__skId)).toBe("true");
    expect(el.value).toBe("true");
    expect(seen).toEqual(["input:true:true", "change:true", "change@form"]);
    expect(descriptor.get!.call(el)).toBe("true");
  });

  test("BubbleSelect: HTMLSelectElement.prototype's value setter, then change", () => {
    const rt = createRuntime();
    const select = rt.nodeOps.createElement("select") as unknown as El;
    for (const v of ["", "apple", "pear"]) {
      const o = rt.nodeOps.createElement("option");
      rt.patchProp(o, "value", null, v);
      rt.nodeOps.insert(o, mirror(select), null);
    }
    const seen: string[] = [];
    select.addEventListener("change", (e) => seen.push(`change:${e.target.value}`));
    const setValue = Object.getOwnPropertyDescriptor(proto(rt, "HTMLSelectElement"), "value")!.set!;
    setValue.call(select, "apple");
    select.dispatchEvent(new Event("change", { bubbles: true }));
    expect(lastValue(rt, select.__skId, ":selected")).toBe("1");
    expect(seen).toEqual(["change:apple"]);
    setValue.call(select, "plum");
    expect(select.value).toBe("");
    expect(lastValue(rt, select.__skId, ":selected")).toBe("-1");
  });

  test("the prototype accessors refuse a receiver of the wrong kind", () => {
    const rt = createRuntime();
    const div = rt.nodeOps.createElement("div");
    const d = Object.getOwnPropertyDescriptor(proto(rt, "HTMLInputElement"), "value")!;
    expect(() => d.set!.call(div, "x")).toThrow();
    expect(() => d.get!.call({})).toThrow();
    expect(() => d.get!.call(Object.create(proto(rt, "HTMLInputElement")))).toThrow();
    const ta = rt.nodeOps.createElement("textarea") as unknown as El;
    Object.getOwnPropertyDescriptor(proto(rt, "HTMLTextAreaElement"), "value")!.set!.call(ta, "hi");
    expect(lastValue(rt, ta.__skId)).toBe("hi");
  });
});

describe("value", () => {
  test("assigning from JS reaches the host every time, and fires no event", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input") as unknown as El;
    const events: string[] = [];
    el.addEventListener("input", () => events.push("input"));
    el.addEventListener("change", () => events.push("change"));
    el.value = "a";
    el.value = "a";
    el.value = null;
    const writes = rt.host.props.filter((p: { id: number; key: string }) => p.id === el.__skId && p.key === "value");
    expect(writes.map((p: { value: unknown }) => p.value)).toEqual(["a", "a", ""]);
    expect(el.value).toBe("");
    expect(events).toEqual([]);
  });

  test("it reads what the user typed, with no @input anywhere", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input") as unknown as El;
    expect(rt.flagsFor(rt.EV_CODES.input).at(-1)).toEqual({ id: el.__skId, code: rt.EV_CODES.input, on: true });
    rt.host.dispatcher(el.__skId, rt.EV_CODES.input, 0, 0, 0, 0, "typed");
    expect(el.value).toBe("typed");
    expect(lastValue(rt, el.__skId)).toBeUndefined();
  });

  test(":value from the template still goes through the host once", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input") as unknown as El;
    rt.patchProp(mirror(el), "value", null, "x");
    expect(el.value).toBe("x");
    expect(
      rt.host.props.filter((p: { id: number; key: string }) => p.id === el.__skId && p.key === "value").length,
    ).toBe(1);
  });

  test("v-model: typing updates the model, and a model reset reaches the field", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input") as unknown as El;
    let model: unknown = null;
    rt.vModelText.created(
      mirror(el),
      { modifiers: {} },
      { props: { "onUpdate:modelValue": (v: unknown) => (model = v) } },
    );
    rt.vModelText.mounted(mirror(el), { value: "" });
    rt.host.dispatcher(el.__skId, rt.EV_CODES.input, 0, 0, 0, 0, "abc");
    expect(model).toBe("abc");
    expect(el.value).toBe("abc");
    rt.vModelText.beforeUpdate(mirror(el), { value: "" }, { props: { "onUpdate:modelValue": () => {} } });
    expect(lastValue(rt, el.__skId)).toBe("");
    expect(el.value).toBe("");
  });

  test("a non-control keeps value as a plain expando, and a file input keeps its refusal", () => {
    const rt = createRuntime();
    const div = rt.nodeOps.createElement("div") as unknown as El;
    const obj = { a: 1 };
    div.value = obj;
    expect(div.value).toBe(obj);
    expect(rt.host.props.some((p: { id: number; key: string }) => p.id === div.__skId && p.key === "value")).toBe(
      false,
    );

    const file = rt.nodeOps.createElement("input") as unknown as El;
    rt.patchProp(mirror(file), "type", null, "file");
    expect(() => {
      file.value = "C:\\x.txt";
    }).toThrow();
    const d = Object.getOwnPropertyDescriptor(proto(rt, "HTMLInputElement"), "value")!;
    expect(() => d.set!.call(file, "C:\\x.txt")).toThrow();
    expect(d.get!.call(file)).toBe("");
  });
});

describe("checked, defaultValue, defaultChecked", () => {
  test("checked follows the attribute until assigned, and an assignment reaches the host", () => {
    const rt = createRuntime();
    const box = rt.nodeOps.createElement("input") as unknown as El;
    rt.patchProp(mirror(box), "type", null, "checkbox");
    expect(box.checked).toBe(false);
    rt.patchProp(mirror(box), "checked", null, true);
    expect(box.checked).toBe(true);
    Object.getOwnPropertyDescriptor(proto(rt, "HTMLInputElement"), "checked")!.set!.call(box, false);
    expect(box.checked).toBe(false);
    expect(lastValue(rt, box.__skId, "checked")).toBe("false");
    expect(box.defaultChecked).toBe(true);
  });

  test("defaultValue is the value attribute (input) or the text (textarea)", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input") as unknown as El;
    expect(el.defaultValue).toBe("");
    el.defaultValue = "d";
    expect(el.attrs.value).toBe("d");
    expect(el.defaultValue).toBe("d");
    const ta = rt.nodeOps.createElement("textarea") as unknown as El;
    rt.nodeOps.setElementText(ta as never, "body");
    expect(ta.defaultValue).toBe("body");
  });
});
