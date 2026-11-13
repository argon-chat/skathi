// `<select>` and `<option>`: the selection model, the host's pick and `v-model`. The host draws the select from
// the reserved `:selected` prop and reports a pick as the select's `input` host event with the option index in `a`.

import { describe, expect, test } from "bun:test";
import { h, nextTick, ref, withDirectives } from "@vue/runtime-core";
import { createRuntime, type MirrorElement } from "./runtime-harness";

type Rt = ReturnType<typeof createRuntime>;
interface SelectApi {
  selectedIndex: number;
  selected: boolean;
  defaultSelected: boolean;
  label: string;
  index: number;
  length: number;
  options: El[];
  selectedOptions: El[];
  type: string;
}
type El = MirrorElement & SelectApi;

const make = (rt: Rt, tag: string) => rt.nodeOps.createElement(tag) as El;

const shown = (rt: Rt, id: number) =>
  rt.host.props.filter((p: { id: number; key: string }) => p.id === id && p.key === ":selected").at(-1)?.value;

function build(rt: Rt, values: (string | null)[], texts?: string[]): { select: El; opts: El[] } {
  const select = make(rt, "select");
  const opts = values.map((v, i) => {
    const o = make(rt, "option");
    if (v != null) rt.patchProp(o, "value", null, v);
    rt.nodeOps.setElementText(o, texts?.[i] ?? `Option ${i}`);
    rt.nodeOps.insert(o, select, null);
    return o;
  });
  return { select, opts };
}

const pick = (rt: Rt, select: El, ix: number) => rt.host.dispatcher(select.__vnId, rt.EV_CODES.input, ix, 0, 0, 0);

describe("the selection model", () => {
  test("a drop-down selects its first enabled option, and the host is told", () => {
    const rt = createRuntime();
    const select = make(rt, "select");
    const off = make(rt, "option");
    rt.patchProp(off, "disabled", null, true);
    rt.nodeOps.insert(off, select, null);
    expect(select.selectedIndex).toBe(-1);
    expect(shown(rt, select.__vnId)).toBe("-1");
    const on = make(rt, "option");
    rt.nodeOps.setElementText(on, "  Two\n words ");
    rt.nodeOps.insert(on, select, null);
    expect(select.selectedIndex).toBe(1);
    expect(on.selected).toBe(true);
    expect(shown(rt, select.__vnId)).toBe("1");
    expect(select.value).toBe("Two words");
    expect([on.text, on.label, on.index, off.index]).toEqual(["Two words", "Two words", 1, 0]);
  });

  test("selected, selectedIndex and value select one option of a single select", () => {
    const rt = createRuntime();
    const { select, opts } = build(rt, ["a", "b", "c"]);
    expect(select.value).toBe("a");
    opts[2]!.selected = true;
    expect([opts[0]!.selected, opts[2]!.selected, select.selectedIndex]).toEqual([false, true, 2]);
    select.selectedIndex = 1;
    expect(select.value).toBe("b");
    select.value = "c";
    expect(select.selectedIndex).toBe(2);
    expect(shown(rt, select.__vnId)).toBe("2");
    select.value = "zzz";
    expect([select.selectedIndex, select.value]).toEqual([-1, ""]);
    select.selectedIndex = 0;
    select.selectedIndex = -1;
    expect(select.selectedIndex).toBe(-1);
    opts[1]!.selected = true;
    opts[1]!.selected = false;
    expect(select.selectedIndex).toBe(0);
    expect(select.length).toBe(3);
    expect(select.options.length).toBe(3);
    expect(select.options[2]).toBe(opts[2]!);
    expect(Array.from(select.selectedOptions)).toEqual([opts[0]!]);
    expect(select.type).toBe("select-one");
  });

  test("the option list follows inserts, moves and removals, optgroups included", () => {
    const rt = createRuntime();
    const { select, opts } = build(rt, ["a", "b"]);
    opts[1]!.selected = true;
    const group = make(rt, "optgroup");
    rt.patchProp(group, "label", null, "G");
    const inner = make(rt, "option");
    rt.patchProp(inner, "value", null, "g1");
    rt.nodeOps.insert(inner, group, null);
    rt.nodeOps.insert(group, select, opts[0]!);
    expect(Array.from(select.options).map((o: El) => o.value)).toEqual(["g1", "a", "b"]);
    expect(select.selectedIndex).toBe(2);
    expect(shown(rt, select.__vnId)).toBe("2");
    rt.nodeOps.remove(opts[1]!);
    expect(select.value).toBe("g1");
    expect(shown(rt, select.__vnId)).toBe("0");
    rt.patchProp(group, "disabled", null, true);
    rt.nodeOps.remove(opts[0]!);
    select.selectedIndex = -1;
    rt.nodeOps.insert(make(rt, "hr"), select, null);
    expect(select.selectedIndex).toBe(-1);
  });

  test("the selected attribute selects an option nothing has selected by hand", () => {
    const rt = createRuntime();
    const { select, opts } = build(rt, ["a", "b", "c"]);
    opts[2]!.setAttribute("selected", "");
    expect(select.selectedIndex).toBe(2);
    expect(opts[2]!.defaultSelected).toBe(true);
    opts[1]!.selected = true;
    opts[1]!.removeAttribute("selected");
    opts[1]!.setAttribute("selected", "");
    expect(select.selectedIndex).toBe(1);
  });
});

describe("the host's pick", () => {
  test("selects the option, then fires input and change, which bubble", () => {
    const rt = createRuntime();
    const form = make(rt, "form");
    const { select } = build(rt, ["a", "b", "c"]);
    rt.nodeOps.insert(select, form, null);
    const seen: string[] = [];
    select.addEventListener("input", (e) =>
      seen.push(`input:${(e.target as El).value}:${e.bubbles}:${(e as unknown as { composed: boolean }).composed}`),
    );
    select.addEventListener("change", (e) => seen.push(`change:${(e.target as El).value}:${e.bubbles}`));
    form.addEventListener("change", () => seen.push("change@form"));
    pick(rt, select, 2);
    expect(seen).toEqual(["input:c:true:true", "change:c:true", "change@form"]);
    expect(shown(rt, select.__vnId)).toBe("2");
    pick(rt, select, 2);
    expect(seen.length).toBe(3);
  });

  test("a disabled option, a disabled select or no such option changes nothing", () => {
    const rt = createRuntime();
    const { select, opts } = build(rt, ["a", "b"]);
    const seen: string[] = [];
    select.addEventListener("change", () => seen.push("change"));
    rt.patchProp(opts[1]!, "disabled", null, true);
    pick(rt, select, 1);
    pick(rt, select, 7);
    rt.patchProp(opts[1]!, "disabled", true, false);
    rt.patchProp(select, "disabled", null, true);
    pick(rt, select, 1);
    expect(seen).toEqual([]);
    expect(select.selectedIndex).toBe(0);
  });

  test("an @change on a select is not told it never fires", () => {
    const rt = createRuntime();
    const select = make(rt, "select");
    rt.patchProp(select, "onChange", null, () => {});
    const div = make(rt, "div");
    rt.patchProp(div, "onChange", null, () => {});
    return Promise.resolve().then(() => {
      const advice = rt.advisories().filter((a: string) => a.includes("@change"));
      expect(advice.length).toBe(1);
      expect(advice[0]).toContain("<select>");
    });
  });
});

describe("v-model", () => {
  function mount(
    rt: Rt,
    model: { value: unknown },
    options: [unknown, string][],
    mods: Record<string, boolean> = {},
    multiple = false,
  ) {
    let el!: El;
    rt.createApp({
      render: () =>
        withDirectives(
          h(
            "select",
            {
              ref: (e: unknown) => (el = e as El),
              multiple: multiple || undefined,
              "onUpdate:modelValue": (v: unknown) => (model.value = v),
            },
            options.map(([value, text]) => h("option", { value }, text)),
          ),
          [[rt.vModelSelect as never, model.value, undefined, mods]],
        ),
    }).mount();
    return el;
  }

  test(".number with numeric :value — the Argon client's settings selects", async () => {
    const rt = createRuntime();
    const model = ref<unknown>(1);
    const options: [unknown, string][] = [
      [0, "Everybody"],
      [1, "Contacts"],
      [2, "Nobody"],
    ];
    let el!: El;
    rt.createApp({
      render: () =>
        withDirectives(
          h(
            "select",
            { ref: (e: unknown) => (el = e as El), "onUpdate:modelValue": (v: unknown) => (model.value = v) },
            options.map(([value, text]) => h("option", { value }, text)),
          ),
          [[rt.vModelSelect as never, model.value, undefined, { number: true }]],
        ),
    }).mount();
    expect(el.selectedIndex).toBe(1);
    expect(shown(rt, el.__vnId)).toBe("1");
    pick(rt, el, 2);
    expect(model.value).toBe(2);
    await nextTick();
    expect(el.selectedIndex).toBe(2);
    model.value = 0;
    await nextTick();
    expect(el.selectedIndex).toBe(0);
    expect(shown(rt, el.__vnId)).toBe("0");
  });

  test("without .number a bound value keeps its type, and an object is matched loosely", async () => {
    const rt = createRuntime();
    const a = { id: 1 };
    const model = { value: { id: 2 } as unknown };
    const el = mount(rt, model, [
      [a, "A"],
      [{ id: 2 }, "B"],
      [3, "C"],
    ]);
    expect(el.selectedIndex).toBe(1);
    pick(rt, el, 2);
    expect(model.value).toBe(3);
    pick(rt, el, 0);
    expect(model.value).toBe(a);
    const rt2 = createRuntime();
    const m2 = { value: "Pear" as unknown };
    let el2!: El;
    rt2
      .createApp({
        render: () =>
          withDirectives(
            h(
              "select",
              { ref: (e: unknown) => (el2 = e as El), "onUpdate:modelValue": (v: unknown) => (m2.value = v) },
              [h("option", "Apple"), h("option", "Pear")],
            ),
            [[rt2.vModelSelect as never, m2.value]],
          ),
      })
      .mount();
    expect(el2.selectedIndex).toBe(1);
    pick(rt2, el2, 0);
    expect(m2.value).toBe("Apple");
  });

  test("a model value no option has selects nothing", () => {
    const rt = createRuntime();
    const el = mount(rt, { value: "zzz" }, [
      ["a", "A"],
      ["b", "B"],
    ]);
    expect(el.selectedIndex).toBe(-1);
    expect(shown(rt, el.__vnId)).toBe("-1");
  });

  test("multiple: an array or a Set of the selected values", () => {
    const rt = createRuntime();
    const model = { value: ["b", "c"] as unknown };
    const el = mount(
      rt,
      model,
      [
        ["a", "A"],
        ["b", "B"],
        ["c", "C"],
      ],
      {},
      true,
    );
    expect(Array.from(el.selectedOptions).map((o: El) => o.value)).toEqual(["b", "c"]);
    expect(el.type).toBe("select-multiple");
    expect(rt.advisories().some((a: string) => a.includes("list box"))).toBe(true);
    pick(rt, el, 0);
    expect(model.value).toEqual(["a"]);
    const rt2 = createRuntime();
    const m2 = { value: new Set(["a"]) as unknown };
    const el2 = mount(
      rt2,
      m2,
      [
        ["a", "A"],
        ["b", "B"],
      ],
      {},
      true,
    );
    expect(el2.selectedIndex).toBe(0);
    pick(rt2, el2, 1);
    expect(m2.value).toEqual(new Set(["b"]));
  });

  test(":value on the select, and @change reading the target — OverlayControls' shape", () => {
    const rt = createRuntime();
    const seen: string[] = [];
    let el!: El;
    rt.createApp({
      render: () =>
        h(
          "select",
          {
            ref: (e: unknown) => (el = e as El),
            value: "screen",
            onChange: (e: { target: El }) => seen.push(e.target.value),
          },
          [h("option", { value: "container" }, "Container"), h("option", { value: "screen" }, "Screen")],
        ),
    }).mount();
    expect(el.value).toBe("screen");
    expect(shown(rt, el.__vnId)).toBe("1");
    pick(rt, el, 0);
    expect(seen).toEqual(["container"]);
  });
});
