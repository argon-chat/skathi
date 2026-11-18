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
  for (const name of ["input", "change", "showpicker"])
    rt.patchProp(
      root,
      "on" + name[0]!.toUpperCase() + name.slice(1),
      null,
      (event: { target: { value: string }; bubbles: boolean; cancelable: boolean }) =>
        seen.push(name + ":" + event.target.value + ":" + event.bubbles + ":" + event.cancelable),
    );
  const opened = () => rt.host.colorPickerCalls;
  const dialogs = () => rt.host.desktop.fs.calls.filter((c: { op: string }) => c.op === "pickColor");
  const click = (node: MirrorElement = input) => {
    rt.host.dispatcher(node.__skId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined);
    rt.host.dispatcher(node.__skId, rt.EV_CODES.click, 0, 0, 0, 0, undefined);
  };
  /** The in-window picker's writes: `commit` is its `b = 1`. */
  const edit = (value: string, commit = false) =>
    rt.host.dispatcher(input.__skId, rt.EV_CODES.input, 0, commit ? 1 : 0, 0, 0, value);
  const answer = async (result: Record<string, unknown>) => {
    rt.host.desktop.settle(dialogs().at(-1)!.token!, true, result);
    await tick();
  };
  return { rt, root, input, seen, opened, dialogs, click, edit, answer };
}

describe("color inputs", () => {
  test("value is the host's sanitization, pushed to the host, and assignment fires nothing", async () => {
    const { rt, input, seen } = world();
    expect(input.value).toBe("#000000");
    input.value = "#AB12EF";
    expect(input.value).toBe("#ab12ef");
    expect(rt.host.props.filter((p: any) => p.key === "value").at(-1)?.value).toBe("#ab12ef");
    (input as { value: unknown }).value = null;
    expect(input.value).toBe("#000000");
    await tick();
    expect(seen).toEqual([]);
  });

  test("the alpha attribute re-sanitizes the value", () => {
    const { rt, input } = world();
    rt.patchProp(input, "alpha", null, "");
    input.value = "#11223344";
    expect(input.value).toBe("#11223344");
    rt.patchProp(input, "alpha", "", null);
    expect(input.value).toBe("#112233");
  });

  test("a click fires a cancelable showpicker, then opens the in-window picker with the gesture", () => {
    const { rt, input, seen, opened, dialogs, click } = world();
    input.value = "#123456";
    click();
    expect(seen).toEqual(["showpicker:#123456:true:true"]);
    expect(opened()).toEqual([input.__skId]);
    expect(rt.activation.isActive()).toBe(false);
    expect(dialogs()).toEqual([]);
  });

  test("the picker's edits fire input, its commit change, and v-model-style listeners read the value", () => {
    const { input, seen, click, edit } = world();
    click();
    edit("#aa0000");
    edit("#bb0000");
    expect(input.value).toBe("#bb0000");
    edit("#bb0000", true);
    expect(seen.slice(1)).toEqual([
      "input:#aa0000:true:false",
      "input:#bb0000:true:false",
      "change:#bb0000:true:false",
    ]);
  });

  test("preventDefault on showpicker keeps every picker closed", () => {
    const { rt, root, input, opened, dialogs, click } = world();
    rt.patchProp(root, "onShowpicker", null, (e: Event) => e.preventDefault());
    click();
    rt.noteActivation("keydown", "Enter");
    input.showPicker();
    expect(opened()).toEqual([]);
    expect(dialogs()).toEqual([]);
  });

  test("with the OS dialog configured, its pick sets value before input and change; cancelling fires nothing", async () => {
    const { rt, input, seen, dialogs, click, answer } = world();
    rt.host.colorPickerAnswer = false;
    input.value = "#123456";
    click();
    expect(dialogs()[0]!.args).toEqual(["#123456"]);
    click();
    expect(dialogs().length).toBe(1);
    await answer({ color: "#AABBCC" });
    expect(input.value).toBe("#aabbcc");
    expect(seen.slice(1)).toEqual(["input:#aabbcc:true:false", "change:#aabbcc:true:false"]);
    rt.noteActivation("keydown", "Enter");
    click();
    await answer({ cancelled: true });
    expect(input.value).toBe("#aabbcc");
    expect(seen.length).toBe(4);
  });

  test("capture preventDefault stops label/drag activation and a disabled input cannot open", () => {
    const { rt, root, input, click, opened } = world();
    rt.patchProp(root, "onClickCapture", null, (e: Event) => e.preventDefault());
    click();
    expect(opened()).toEqual([]);
    rt.patchProp(root, "onClickCapture", null, null);
    rt.patchProp(input, "disabled", null, "");
    click();
    input.click();
    expect(opened()).toEqual([]);
  });

  test("a hidden input clicked by a real button uses the same default action", () => {
    const { rt, root, input, click, opened } = world();
    const button = rt.nodeOps.createElement("button");
    rt.nodeOps.insert(button, root, null);
    rt.patchProp(input, "style", null, { display: "none" });
    rt.patchProp(button, "onClick", null, () => input.click());
    click(button);
    expect(opened()).toEqual([input.__skId]);
  });

  test("showPicker requires activation and honors disabled but not readonly", () => {
    const { rt, input, opened } = world();
    expect(() => input.showPicker()).toThrow();
    rt.patchProp(input, "readonly", null, "");
    rt.noteActivation("keydown", "Enter");
    input.showPicker();
    expect(opened().length).toBe(1);
    rt.patchProp(input, "disabled", null, "");
    expect(() => input.showPicker()).toThrow();
  });

  test("disabled fieldset suppresses click and showPicker; a first legend follows HTML's exemption", () => {
    const { rt, root, input, click, opened } = world();
    const fieldset = rt.nodeOps.createElement("fieldset");
    rt.nodeOps.insert(fieldset, root, null);
    rt.nodeOps.insert(input, fieldset, null);
    rt.patchProp(fieldset, "disabled", null, "");
    click();
    input.click();
    expect(() => input.showPicker()).toThrow();
    expect(opened()).toEqual([]);
    const legend = rt.nodeOps.createElement("legend");
    rt.nodeOps.insert(legend, fieldset, null);
    rt.nodeOps.insert(input, legend, null);
    click();
    expect(opened().length).toBe(1);
  });

  test("an unmounted color input does not overwrite a reused native node on a late OS dialog answer", async () => {
    const { rt, input, click, answer, seen } = world();
    rt.host.colorPickerAnswer = false;
    input.value = "#112233";
    click();
    rt.nodeOps.remove(input);
    await answer({ color: "#abcdef" });
    expect(input.value).toBe("#112233");
    expect(seen.filter((s) => !s.startsWith("showpicker"))).toEqual([]);
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
