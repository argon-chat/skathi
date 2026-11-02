// Form-control `value` / `checked` as real accessors (m26 fixes, URGENT 1).
//
// reka-ui's VisuallyHiddenInputBubble (Checkbox / Switch / RadioGroup in a
// form) and BubbleSelect set a control's value through the PROTOTYPE's
// descriptor and dispatch `input` / `change` themselves. With tag classes that
// had no `value` member that was `undefined.set` — the Argon client crashed
// right after sign-in ("Object.getOwnPropertyDescriptor(u,"value").set").

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

type El = {
  id: number;
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
const proto = (rt: Rt, name: string) => (rt.global as Record<string, { prototype: object }>)[name].prototype;

describe("the reka bubble-input pattern", () => {
  test("HTMLInputElement.prototype's value setter reaches the host, and the hand-made events are heard", () => {
    const rt = createRuntime();
    const root = rt.nodeOps.createElement("form") as unknown as El;
    const el = rt.nodeOps.createElement("input") as unknown as El;
    rt.nodeOps.insert(el as never, root as never, null);
    const seen: string[] = [];
    el.addEventListener("input", (e) => seen.push(`input:${e.target.value}:${e.bubbles}`));
    el.addEventListener("change", (e) => seen.push(`change:${e.target.value}`));
    root.addEventListener("change", () => seen.push("change@form"));

    // VisuallyHiddenInputBubble.js, verbatim but for `window`.
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

    expect(lastValue(rt, el.__vnId)).toBe("true");
    expect(el.value).toBe("true");
    expect(seen).toEqual(["input:true:true", "change:true", "change@form"]);
    expect(descriptor.get!.call(el)).toBe("true");
  });

  test("BubbleSelect: HTMLSelectElement.prototype's value setter, then change", () => {
    const rt = createRuntime();
    const select = rt.nodeOps.createElement("select") as unknown as El;
    // reka's BubbleSelect renders an empty option and one per item.
    for (const v of ["", "apple", "pear"]) {
      const o = rt.nodeOps.createElement("option");
      rt.patchProp(o, "value", null, v);
      rt.nodeOps.insert(o as never, select as never, null);
    }
    const seen: string[] = [];
    select.addEventListener("change", (e) => seen.push(`change:${e.target.value}`));
    const setValue = Object.getOwnPropertyDescriptor(proto(rt, "HTMLSelectElement"), "value")!.set!;
    setValue.call(select, "apple");
    select.dispatchEvent(new Event("change", { bubbles: true }));
    // The host draws the option it selected.
    expect(lastValue(rt, select.__vnId, ":selected")).toBe("1");
    expect(seen).toEqual(["change:apple"]);
    // No option has the value: nothing is selected, and the value reads "".
    setValue.call(select, "plum");
    expect(select.value).toBe("");
    expect(lastValue(rt, select.__vnId, ":selected")).toBe("-1");
  });

  test("the prototype accessors refuse a receiver of the wrong kind", () => {
    const rt = createRuntime();
    const div = rt.nodeOps.createElement("div");
    const d = Object.getOwnPropertyDescriptor(proto(rt, "HTMLInputElement"), "value")!;
    expect(() => d.set!.call(div, "x")).toThrow("Illegal invocation");
    expect(() => d.get!.call({})).toThrow("Illegal invocation");
    expect(() => d.get!.call(Object.create(proto(rt, "HTMLInputElement")))).toThrow("Illegal invocation");
    const ta = rt.nodeOps.createElement("textarea") as unknown as El;
    Object.getOwnPropertyDescriptor(proto(rt, "HTMLTextAreaElement"), "value")!.set!.call(ta, "hi");
    expect(lastValue(rt, ta.__vnId)).toBe("hi");
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
    const writes = rt.host.props.filter((p: { id: number; key: string }) => p.id === el.__vnId && p.key === "value");
    expect(writes.map((p: { value: unknown }) => p.value)).toEqual(["a", "a", ""]);
    expect(el.value).toBe("");
    expect(events).toEqual([]);
  });

  test("it reads what the user typed, with no @input anywhere", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input") as unknown as El;
    // The host delivers `input` to a text control on its own account…
    expect(rt.flagsFor(rt.EV_CODES.input).at(-1)).toEqual({ id: el.__vnId, code: rt.EV_CODES.input, on: true });
    // …and the payload is the field's new value.
    rt.host.dispatcher(el.__vnId, rt.EV_CODES.input, 0, 0, 0, 0, "typed");
    expect(el.value).toBe("typed");
    // The echo is not pushed back to the field it came from.
    expect(lastValue(rt, el.__vnId)).toBeUndefined();
  });

  test(":value from the template still goes through the host once", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input") as unknown as El;
    rt.patchProp(el, "value", null, "x");
    expect(el.value).toBe("x");
    expect(rt.host.props.filter((p: { id: number; key: string }) => p.id === el.__vnId && p.key === "value").length).toBe(1);
  });

  test("v-model: typing updates the model, and a model reset reaches the field", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input") as unknown as El;
    let model: unknown = null;
    rt.vModelText.created(el, { modifiers: {} }, { props: { "onUpdate:modelValue": (v: unknown) => (model = v) } });
    rt.vModelText.mounted(el, { value: "" });
    rt.host.dispatcher(el.__vnId, rt.EV_CODES.input, 0, 0, 0, 0, "abc");
    expect(model).toBe("abc");
    expect(el.value).toBe("abc");
    // The model is cleared: the field held "abc", so "" is written.
    rt.vModelText.beforeUpdate(el, { value: "" }, { props: { "onUpdate:modelValue": () => {} } });
    expect(lastValue(rt, el.__vnId)).toBe("");
    expect(el.value).toBe("");
  });

  test("a non-control keeps value as a plain expando, and a file input keeps its refusal", () => {
    const rt = createRuntime();
    const div = rt.nodeOps.createElement("div") as unknown as El;
    const obj = { a: 1 };
    div.value = obj;
    expect(div.value).toBe(obj);
    expect(rt.host.props.some((p: { id: number; key: string }) => p.id === div.__vnId && p.key === "value")).toBe(false);

    const file = rt.nodeOps.createElement("input") as unknown as El;
    rt.patchProp(file, "type", null, "file");
    expect(() => {
      file.value = "C:\\x.txt";
    }).toThrow("may only be programmatically set to the empty string");
    const d = Object.getOwnPropertyDescriptor(proto(rt, "HTMLInputElement"), "value")!;
    expect(() => d.set!.call(file, "C:\\x.txt")).toThrow("may only be programmatically set to the empty string");
    expect(d.get!.call(file)).toBe("");
  });
});

describe("checked, defaultValue, defaultChecked", () => {
  test("checked follows the attribute until assigned, and an assignment reaches the host", () => {
    const rt = createRuntime();
    const box = rt.nodeOps.createElement("input") as unknown as El;
    rt.patchProp(box, "type", null, "checkbox");
    expect(box.checked).toBe(false);
    rt.patchProp(box, "checked", null, true);
    expect(box.checked).toBe(true);
    Object.getOwnPropertyDescriptor(proto(rt, "HTMLInputElement"), "checked")!.set!.call(box, false);
    expect(box.checked).toBe(false);
    expect(lastValue(rt, box.__vnId, "checked")).toBe("false");
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
