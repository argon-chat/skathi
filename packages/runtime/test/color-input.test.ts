import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
function world() {
  const rt = createRuntime();
  const root = rt.nodeOps.createElement("div");
  const input = rt.nodeOps.createElement("input") as MirrorElement & { type: string };
  rt.nodeOps.insert(input, root, null);
  rt.patchProp(input, "type", null, "color");
  const seen: string[] = [];
  for (const name of ["input", "change"])
    rt.patchProp(
      root,
      "on" + name[0]!.toUpperCase() + name.slice(1),
      null,
      (event: { target: { value: string }; bubbles: boolean; composed: boolean }) =>
        seen.push(name + ":" + event.target.value + ":" + event.bubbles + ":" + event.composed),
    );
  const calls = () => rt.host.desktop.fs.calls.filter((c: { op: string }) => c.op === "pickColor");
  const click = (node: MirrorElement = input) => {
    rt.host.dispatcher(node.__skId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
    rt.host.dispatcher(node.__skId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
  };
  const answer = async (result: Record<string, unknown>) => {
    rt.host.desktop.settle(calls().at(-1)!.token!, true, result);
    await tick();
  };
  return { rt, root, input, seen, calls, click, answer };
}

describe("native color inputs", () => {
  test("value defaults to black and normalizes assignment without firing events", async () => {
    const { rt, input, seen } = world();
    expect(input.value).toBe("#000000");
    input.value = "#AB12EF";
    expect(input.value).toBe("#ab12ef");
    expect(rt.host.props.filter((p: any) => p.key === "value").at(-1)?.value).toBe("#ab12ef");
    input.value = "#f00";
    expect(input.value).toBe("#ff0000");
    input.value = "invalid";
    expect(input.value).toBe("#000000");
    (input as { value: unknown }).value = null;
    expect(input.value).toBe("#000000");
    await tick();
    expect(seen).toEqual([]);
    expect(rt.advisories().filter((s: string) => s.includes("@change"))).toEqual([]);
  });

  test("an uncanceled native click opens once, then sets value before input and change", async () => {
    const { rt, input, seen, calls, click, answer } = world();
    input.value = "#123456";
    click();
    expect(calls()[0]!.args).toEqual(["#123456"]);
    expect(rt.activation.isActive()).toBe(false);
    click();
    expect(calls().length).toBe(1);
    await answer({ color: "#AABBCC" });
    expect(input.value).toBe("#aabbcc");
    expect(seen).toEqual(["input:#aabbcc:true:true", "change:#aabbcc:true:false"]);
  });

  test("cancel and selecting the current color preserve value and emit no edit events", async () => {
    const { input, seen, click, answer } = world();
    input.value = "#123456";
    click();
    await answer({ cancelled: true });
    expect(input.value).toBe("#123456");
    click();
    await answer({ color: "#123456" });
    expect(seen).toEqual([]);
  });

  test("capture preventDefault stops label/drag activation and disabled input cannot open", () => {
    const { rt, root, input, click, calls } = world();
    rt.patchProp(root, "onClickCapture", null, (e: Event) => e.preventDefault());
    click();
    expect(calls()).toEqual([]);
    rt.patchProp(root, "onClickCapture", null, null);
    rt.patchProp(input, "disabled", null, "");
    click();
    input.click();
    expect(calls()).toEqual([]);
  });

  test("a hidden input clicked by a real button uses the same default action", async () => {
    const { rt, root, input, click, calls, answer, seen } = world();
    const button = rt.nodeOps.createElement("button");
    rt.nodeOps.insert(button, root, null);
    rt.patchProp(input, "style", null, { display: "none" });
    rt.patchProp(button, "onClick", null, () => input.click());
    click(button);
    expect(calls().length).toBe(1);
    await answer({ color: "#abcdef" });
    expect(seen).toEqual(["input:#abcdef:true:true", "change:#abcdef:true:false"]);
  });

  test("showPicker requires activation and honors disabled but not readonly", () => {
    const { rt, input, calls } = world();
    expect(() => input.showPicker()).toThrow("requires a user gesture");
    rt.patchProp(input, "readonly", null, "");
    rt.noteActivation("keydown", "Enter");
    input.showPicker();
    expect(calls().length).toBe(1);
    rt.patchProp(input, "disabled", null, "");
    expect(() => input.showPicker()).toThrow("immutable controls");
  });
  test("unsupported assigned CSS colors advise once; an empty default stays quiet", () => {
    const { rt, input } = world();
    expect(input.value).toBe("#000000");
    input.value = "";
    expect(rt.advisories().filter((line: string) => line.includes("other CSS color values"))).toEqual([]);
    input.value = "red";
    input.value = "rgb(1 2 3)";
    expect(input.value).toBe("#000000");
    expect(rt.advisories().filter((line: string) => line.includes("other CSS color values")).length).toBe(1);
  });

  test("disabled fieldset suppresses click and showPicker; a first legend follows HTML's exemption", () => {
    const { rt, root, input, click, calls } = world();
    const fieldset = rt.nodeOps.createElement("fieldset");
    rt.nodeOps.insert(fieldset, root, null);
    rt.nodeOps.insert(input, fieldset, null);
    rt.patchProp(fieldset, "disabled", null, "");
    click();
    input.click();
    expect(() => input.showPicker()).toThrow("immutable controls");
    expect(calls()).toEqual([]);
    const legend = rt.nodeOps.createElement("legend");
    rt.nodeOps.insert(legend, fieldset, null);
    rt.nodeOps.insert(input, legend, null);
    click();
    expect(calls().length).toBe(1);
  });

  test("an unmounted color input does not overwrite a reused native node on a late picker answer", async () => {
    const { rt, input, click, answer, seen } = world();
    input.value = "#112233";
    click();
    rt.nodeOps.remove(input);
    await answer({ color: "#abcdef" });
    expect(input.value).toBe("#112233");
    expect(seen).toEqual([]);
  });

  test("Vue/attribute value and switching type keep native swatch and live value synchronized", () => {
    const { rt, input } = world();
    rt.patchProp(input, "value", null, "#F0A");
    expect(input.value).toBe("#ff00aa");
    expect(rt.host.props.filter((p: any) => p.key === "value").at(-1)?.value).toBe("#ff00aa");
    input.type = "text";
    input.value = "unsupported";
    input.type = "color";
    expect(input.value).toBe("#000000");
    expect(rt.host.props.filter((p: any) => p.key === "value").at(-1)?.value).toBe("#000000");
  });
});
