// Pointer and image events: `contextmenu`, `auxclick`, `dblclick` (and `detail`), `pointercancel`, an `<img>`'s
// `load` / `error` and the `on<event>` attributes. The host half is crates/vn-tree/src/render.rs and img.rs.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

class FakeMouseEvent extends Event {
  clientX: number;
  clientY: number;
  button: number;
  buttons: number;
  detail: number;
  ctrlKey: boolean;
  shiftKey: boolean;
  constructor(type: string, init: Record<string, unknown> = {}) {
    super(type, init as EventInit);
    this.clientX = (init.clientX as number) ?? 0;
    this.clientY = (init.clientY as number) ?? 0;
    this.button = (init.button as number) ?? 0;
    this.buttons = (init.buttons as number) ?? 0;
    this.detail = (init.detail as number) ?? 0;
    this.ctrlKey = !!init.ctrlKey;
    this.shiftKey = !!init.shiftKey;
  }
}

type Ev = FakeMouseEvent & { pointerType?: string; pointerId?: number; isPrimary?: boolean; pressure?: number };
type Rt = ReturnType<typeof createRuntime>;
type El = {
  id: number;
  attrs: Record<string, string>;
  addEventListener(t: string, f: (e: Ev) => void, o?: unknown): void;
  removeEventListener(t: string, f: (e: Ev) => void): void;
  setAttribute(k: string, v: string): void;
  [k: string]: unknown;
};

function tree(rt: Rt) {
  const root = rt.nodeOps.createElement("root") as unknown as El;
  const outer = rt.nodeOps.createElement("div") as unknown as El;
  const inner = rt.nodeOps.createElement("span") as unknown as El;
  rt.nodeOps.insert(outer as never, root as never, null);
  rt.nodeOps.insert(inner as never, outer as never, null);
  return { root, outer, inner };
}
const fire = (rt: Rt, el: El, name: string, a = 0, b = 0, c = 0, d = 0, key?: string) =>
  rt.host.dispatcher(el.__vnId, (rt.EV_CODES as Record<string, number>)[name], a, b, c, d, key);

describe("contextmenu", () => {
  test("a listener arms the host's own code", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(4), "onContextmenu", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.contextmenu)).toEqual([{ id: 4, code: 42, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("a right-click's: a MouseEvent that bubbles and is cancelable, button 2, buttons 0, detail 0", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { outer, inner } = tree(rt);
    let seen: Ev | undefined;
    const path: string[] = [];
    inner.addEventListener("contextmenu", () => path.push("inner"));
    outer.addEventListener("contextmenu", (e) => {
      seen = e;
      path.push("outer");
    });
    fire(rt, inner, "contextmenu", 30, 40, 2, 1 /* ctrl */);
    expect(path).toEqual(["inner", "outer"]);
    expect(seen).toBeInstanceOf(FakeMouseEvent);
    expect([seen!.type, seen!.bubbles, seen!.cancelable]).toEqual(["contextmenu", true, true]);
    expect([seen!.clientX, seen!.clientY, seen!.button, seen!.buttons, seen!.detail]).toEqual([30, 40, 2, 0, 0]);
    expect(seen!.ctrlKey).toBe(true);
  });

  test("the reka trigger: preventDefault, open its own menu — and the host hears the cancel", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    let open: { x: number; y: number } | null = null;
    rt.patchProp(inner as never, "onContextmenu", null, (e: Ev) => {
      if (e.defaultPrevented) return;
      e.preventDefault();
      open = { x: e.clientX, y: e.clientY };
    });
    expect(fire(rt, inner, "contextmenu", 12, 34, 2)).toBe(false);
    expect(open).toEqual({ x: 12, y: 34 });
    expect(rt.advisories()).toEqual([]);
  });

  test("an uncancelled one says, once, that no native menu opens", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    inner.addEventListener("contextmenu", () => {});
    expect(fire(rt, inner, "contextmenu", 0, 0, 2)).toBe(true);
    fire(rt, inner, "contextmenu", 0, 0, 2);
    expect(rt.advisories().length).toBe(1);
    expect(rt.advisories()[0]).toContain("context menu");
  });

  test("from the keyboard: no button, at the coordinates the host chose", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    let seen: Ev | undefined;
    inner.addEventListener("contextmenu", (e) => (seen = e));
    fire(rt, inner, "contextmenu", 50, 10, 0);
    expect([seen!.button, seen!.buttons, seen!.clientX, seen!.clientY]).toEqual([0, 0, 50, 10]);
  });

  test("the handler attribute works too", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    inner.oncontextmenu = () => false;
    expect(fire(rt, inner, "contextmenu", 0, 0, 2)).toBe(false);
  });
});

describe("auxclick", () => {
  test("a listener arms the host's own code, and is not advised as undelivered", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(6), "onAuxclick", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.auxclick)).toEqual([{ id: 6, code: 47, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("a middle click: a MouseEvent that bubbles and is cancelable, button 1, buttons 0, detail the count", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { outer, inner } = tree(rt);
    let seen: Ev | undefined;
    const path: string[] = [];
    inner.addEventListener("auxclick", () => path.push("inner"));
    outer.addEventListener("auxclick", (e) => {
      seen = e;
      path.push("outer");
    });
    fire(rt, inner, "auxclick", 30, 40, 1, (1 << 4) | 1 /* ctrl */);
    expect(path).toEqual(["inner", "outer"]);
    expect(seen).toBeInstanceOf(FakeMouseEvent);
    expect([seen!.type, seen!.bubbles, seen!.cancelable]).toEqual(["auxclick", true, true]);
    expect([seen!.clientX, seen!.clientY, seen!.button, seen!.buttons, seen!.detail]).toEqual([30, 40, 1, 0, 1]);
    expect(seen!.ctrlKey).toBe(true);
  });

  test("ChannelItem.vue's split: button 1, preventDefault, and the host hears the cancel", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    const split: number[] = [];
    rt.patchProp(inner as never, "onAuxclick", null, (e: Ev) => {
      if (e.button === 1) {
        e.preventDefault();
        split.push(e.button);
      }
    });
    expect(fire(rt, inner, "auxclick", 0, 0, 1, 1 << 4)).toBe(false);
    expect(fire(rt, inner, "auxclick", 0, 0, 2, 1 << 4)).toBe(true);
    expect(split).toEqual([1]);
  });

  test("never a click nor a dblclick, whatever its count; the handler attribute works", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    const seen: string[] = [];
    inner.addEventListener("click", () => seen.push("click"));
    inner.addEventListener("dblclick", () => seen.push("dblclick"));
    inner.onauxclick = (e: Ev) => seen.push(`auxclick:${e.detail}`);
    fire(rt, inner, "auxclick", 0, 0, 1, 1 << 4);
    fire(rt, inner, "auxclick", 0, 0, 1, 2 << 4);
    expect(seen).toEqual(["auxclick:1", "auxclick:2"]);
  });
});

describe("beforeinput, input and the composition events", () => {
  type Edit = Ev & { inputType: string; data: string | null; isComposing: boolean; composed: boolean };
  const send = (rt: Rt, el: El, name: string, type: number, data?: string, composing = 0, value?: string) =>
    rt.host.dispatcher(el.__vnId, (rt.EV_CODES as Record<string, number>)[name], type, composing, 0, 0, value, data);
  const field = (rt: Rt, tag = "textarea") => {
    const { outer } = tree(rt);
    const el = rt.nodeOps.createElement(tag) as unknown as El;
    rt.nodeOps.insert(el as never, outer as never, null);
    return { outer, el };
  };
  const read = (e: Edit) => [e.type, e.inputType, e.data, e.isComposing, e.bubbles, e.cancelable, e.composed];

  test("@beforeinput and the composition events arm their own codes, and are not advised", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(5), "onBeforeinput", null, () => {});
    rt.patchProp(rt.el(5), "onCompositionstart", null, () => {});
    rt.patchProp(rt.el(5), "onCompositionend", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.beforeinput)).toEqual([{ id: 5, code: 48, on: true }]);
    expect(rt.flagsFor(rt.EV_CODES.compositionstart)).toEqual([{ id: 5, code: 49, on: true }]);
    expect(rt.flagsFor(rt.EV_CODES.compositionend)).toEqual([{ id: 5, code: 51, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("typing: beforeinput, then input, both InputEvents with inputType and data; only beforeinput cancels", () => {
    const rt = createRuntime();
    const { outer, el } = field(rt);
    const seen: unknown[][] = [];
    outer.addEventListener("beforeinput", (e) => seen.push([...read(e as Edit), el.value]));
    outer.addEventListener("input", (e) => seen.push([...read(e as Edit), el.value]));
    expect(send(rt, el, "beforeinput", 1, "a")).toBe(true);
    send(rt, el, "input", 1, "a", 0, "a");
    expect(seen).toEqual([
      ["beforeinput", "insertText", "a", false, true, true, true, ""],
      ["input", "insertText", "a", false, true, false, true, "a"],
    ]);
  });

  test("useEmojiSuggest's swallowed space: a cancelled beforeinput answers the host false", () => {
    const rt = createRuntime();
    const { el } = field(rt);
    el.addEventListener("beforeinput", (e) => {
      const ie = e as Edit;
      if (ie.inputType === "insertText" && ie.data === " ") ie.preventDefault();
    });
    expect(send(rt, el, "beforeinput", 1, " ")).toBe(false);
    expect(send(rt, el, "beforeinput", 1, "x")).toBe(true);
    expect(rt.advisories()).toEqual([]);
  });

  test("a deletion and a line break carry no data; a paste carries its text", () => {
    const rt = createRuntime();
    const { el } = field(rt);
    const seen: unknown[] = [];
    el.addEventListener("beforeinput", (e) => seen.push([(e as Edit).inputType, (e as Edit).data]));
    send(rt, el, "beforeinput", 5);
    send(rt, el, "beforeinput", 7);
    send(rt, el, "beforeinput", 2);
    send(rt, el, "beforeinput", 3, "pasted\ntext");
    send(rt, el, "beforeinput", 11);
    send(rt, el, "beforeinput", 12);
    expect(seen).toEqual([
      ["deleteContentBackward", null],
      ["deleteWordBackward", null],
      ["insertLineBreak", null],
      ["insertFromPaste", "pasted\ntext"],
      ["deleteByCut", null],
      ["historyUndo", null],
    ]);
  });

  test("the IME's beforeinput says isComposing and cannot be cancelled", () => {
    const rt = createRuntime();
    const { el } = field(rt);
    let seen: Edit | undefined;
    el.addEventListener("beforeinput", (e) => {
      seen = e as Edit;
      e.preventDefault();
    });
    expect(send(rt, el, "beforeinput", 4, "に", 1)).toBe(true);
    expect(read(seen!)).toEqual(["beforeinput", "insertCompositionText", "に", true, true, false, true]);
    expect(seen!.defaultPrevented).toBe(false);
  });

  test("the composition events are CompositionEvents with data, and bubble", () => {
    const rt = createRuntime();
    const { outer, el } = field(rt);
    const C = (rt.global as unknown as { CompositionEvent: new (t: string) => Event }).CompositionEvent;
    const seen: unknown[] = [];
    for (const t of ["compositionstart", "compositionupdate", "compositionend"])
      outer.addEventListener(t, (e) =>
        seen.push([e.type, (e as unknown as { data: string }).data, e instanceof C, e.bubbles]),
      );
    send(rt, el, "compositionstart", 0, "");
    send(rt, el, "compositionupdate", 0, "か");
    send(rt, el, "compositionend", 0, "火");
    expect(seen).toEqual([
      ["compositionstart", "", true, true],
      ["compositionupdate", "か", true, true],
      ["compositionend", "火", true, true],
    ]);
  });

  test("v-model leaves the model alone while an IME composes, as runtime-dom does", () => {
    const rt = createRuntime();
    const { el } = field(rt, "input");
    const model: unknown[] = [];
    const binding = { modifiers: {} };
    rt.vModelText.created(el as never, binding, { props: { "onUpdate:modelValue": (v: unknown) => model.push(v) } });
    send(rt, el, "input", 1, "a", 0, "a");
    send(rt, el, "compositionstart", 0, "");
    send(rt, el, "input", 4, "k", 1, "ak");
    send(rt, el, "input", 4, "か", 1, "aか");
    expect(model).toEqual(["a"]);
    const writes = () =>
      rt.host.props.filter((p: { id: number; key: string }) => p.id === el.__vnId && p.key === "value").length;
    const before = writes();
    rt.vModelText.beforeUpdate(
      el as never,
      { value: "a" },
      { props: { "onUpdate:modelValue": (v: unknown) => model.push(v) } },
    );
    expect(writes()).toBe(before);
    send(rt, el, "compositionend", 0, "か");
    expect(model).toEqual(["a", "aか"]);
    send(rt, el, "input", 1, "b", 0, "aかb");
    expect(model).toEqual(["a", "aか", "aかb"]);
  });

  test("InputEvent.prototype has a null dataTransfer and no target ranges, as a text field's has", () => {
    class EngineInputEvent extends Event {}
    const rt = createRuntime({ globals: { InputEvent: EngineInputEvent } });
    const ie = new (rt.global as unknown as { InputEvent: typeof EngineInputEvent }).InputEvent(
      "beforeinput",
    ) as unknown as {
      dataTransfer: unknown;
      getTargetRanges(): unknown[];
    };
    expect(ie.dataTransfer).toBe(null);
    expect(ie.getTargetRanges()).toEqual([]);
  });
});

describe("dblclick and detail", () => {
  test("@dblclick arms the click bit — it has no code of its own", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(8), "onDblclick", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: 8, code: 1, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("the second click of a pair fires dblclick after it; detail rides the click", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { outer, inner } = tree(rt);
    const seen: string[] = [];
    outer.addEventListener("click", (e) => seen.push(`click:${e.detail}`));
    outer.addEventListener("dblclick", (e) => seen.push(`dblclick:${e.detail}:${e.bubbles}:${e.button}`));
    fire(rt, inner, "click", 5, 5, 0, 1 << 4);
    fire(rt, inner, "click", 5, 5, 0, 2 << 4);
    fire(rt, inner, "click", 5, 5, 0, 3 << 4);
    expect(seen).toEqual(["click:1", "click:2", "dblclick:2:true:0", "click:3"]);
  });

  test("a keyboard click is detail 0, and never a dblclick", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    const seen: string[] = [];
    inner.addEventListener("click", (e) => seen.push(`click:${e.detail}`));
    inner.addEventListener("dblclick", () => seen.push("dblclick"));
    fire(rt, inner, "click", 0, 0, 0, 0);
    expect(seen).toEqual(["click:0"]);
  });

  test("the modifier bits are still read off the low bits", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    let seen: Ev | undefined;
    inner.addEventListener("mousedown", (e) => (seen = e));
    fire(rt, inner, "mousedown", 0, 0, 0, (2 << 4) | 2 /* shift */);
    expect([seen!.detail, seen!.shiftKey, seen!.ctrlKey]).toEqual([2, true, false]);
  });
});

describe("button and buttons", () => {
  test("a press has its button down; a release, a click or a menu has none", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    const seen: string[] = [];
    for (const t of ["mousedown", "pointerdown", "mouseup", "pointerup"])
      inner.addEventListener(t, (e) => seen.push(`${t}:${e.button}/${e.buttons}`));
    fire(rt, inner, "mousedown", 0, 0, 2);
    fire(rt, inner, "mouseup", 0, 0, 2);
    fire(rt, inner, "mousedown", 0, 0, 1);
    // The DOM's bit order is not the index order: right is 2, middle is 4.
    expect(seen).toEqual([
      "pointerdown:2/2",
      "mousedown:2/2",
      "pointerup:2/0",
      "mouseup:2/0",
      "pointerdown:1/4",
      "mousedown:1/4",
    ]);
  });

  test("a move changes no button: 0 on the mouse event, -1 on the pointer event; buttons is what is held", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    const seen: string[] = [];
    inner.addEventListener("mousemove", (e) => seen.push(`mouse:${e.button}/${e.buttons}`));
    inner.addEventListener("pointermove", (e) => seen.push(`pointer:${e.button}/${e.buttons}`));
    fire(rt, inner, "mousemove", 0, 0, -1);
    fire(rt, inner, "mousemove", 0, 0, 0);
    expect(seen).toEqual(["pointer:-1/0", "mouse:0/0", "pointer:-1/1", "mouse:0/1"]);
  });
});

describe("a cancelled press", () => {
  test("the host is told: a cancelled mousedown or pointerdown returns false", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    inner.addEventListener("mousedown", (e) => e.preventDefault());
    expect(fire(rt, inner, "mousedown", 0, 0, 0)).toBe(false);
    const { inner: other } = tree(rt);
    other.addEventListener("pointerdown", (e) => e.preventDefault());
    expect(fire(rt, other, "mousedown", 0, 0, 0)).toBe(false);
    expect(rt.advisories()).toEqual([]);
  });

  test("on a text field it is advised once: the field has already focused itself", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { outer } = tree(rt);
    const field = rt.nodeOps.createElement("textarea") as unknown as El;
    rt.nodeOps.insert(field as never, outer as never, null);
    field.addEventListener("mousedown", (e) => e.preventDefault());
    expect(fire(rt, field, "mousedown", 0, 0, 0)).toBe(false);
    fire(rt, field, "mousedown", 0, 0, 0);
    expect(rt.advisories().length).toBe(1);
    expect(rt.advisories()[0]).toContain("already focused itself");
  });

  test("an uncancelled press on a field, or a cancelled one on a file input, is not advised", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { outer } = tree(rt);
    const field = rt.nodeOps.createElement("input") as unknown as El;
    rt.nodeOps.insert(field as never, outer as never, null);
    field.addEventListener("mousedown", () => {});
    expect(fire(rt, field, "mousedown", 0, 0, 0)).toBe(true);
    const file = rt.nodeOps.createElement("input") as unknown as El;
    file.setAttribute("type", "file");
    rt.nodeOps.insert(file as never, outer as never, null);
    file.addEventListener("mousedown", (e) => e.preventDefault());
    expect(fire(rt, file, "mousedown", 0, 0, 0)).toBe(false);
    expect(rt.advisories().filter((a) => a.includes("already focused itself"))).toEqual([]);
  });
});

describe("keyboard clicks", () => {
  const keyClick = (rt: Rt, el: El, key: string) =>
    rt.host.dispatcher(el.__vnId as number, rt.EV_CODES.click, 0, 0, 0, 0, key);

  test("a keyboard click is detail 0, and one on a button or a link says nothing", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { outer } = tree(rt);
    const button = rt.nodeOps.createElement("button") as unknown as El;
    const link = rt.nodeOps.createElement("a") as unknown as El;
    link.setAttribute("href", "#x");
    const box = rt.nodeOps.createElement("input") as unknown as El;
    box.setAttribute("type", "checkbox");
    for (const el of [button, link, box]) rt.nodeOps.insert(el as never, outer as never, null);
    const seen: number[] = [];
    for (const el of [button, link, box]) el.addEventListener("click", (e) => seen.push(e.detail));
    keyClick(rt, button, "Enter");
    keyClick(rt, button, " ");
    keyClick(rt, link, "Enter");
    keyClick(rt, box, " ");
    expect(seen).toEqual([0, 0, 0, 0]);
    expect(rt.advisories()).toEqual([]);
  });

  test("a mouse click carries no key and is not advised", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    inner.addEventListener("click", () => {});
    fire(rt, inner, "click", 5, 5, 0, 1 << 4);
    fire(rt, inner, "click", 5, 5, 0, 0);
    expect(rt.advisories()).toEqual([]);
  });
});

describe("passive listeners (a cancelled wheel scrolls nothing)", () => {
  test("@wheel.passive is delivered, and its preventDefault() does nothing", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    const seen: boolean[] = [];
    rt.patchProp(inner as never, "onWheelPassive", null, (e: Ev) => {
      e.preventDefault();
      seen.push(e.defaultPrevented);
    });
    expect(rt.flagsFor(rt.EV_CODES.wheel)).toEqual([{ id: inner.__vnId as number, code: rt.EV_CODES.wheel, on: true }]);
    expect(fire(rt, inner, "wheel", 0, 40)).toBe(true);
    expect(seen).toEqual([false]);
    expect(rt.advisories()).toEqual([]);
  });

  test("an element's wheel listener cancels; one added with { passive: true } does not", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner, outer } = tree(rt);
    inner.addEventListener("wheel", (e) => e.preventDefault(), { passive: true });
    expect(fire(rt, inner, "wheel", 0, 40)).toBe(true);
    outer.addEventListener("wheel", (e) => e.preventDefault());
    expect(fire(rt, inner, "wheel", 0, 40)).toBe(false);
  });

  test("on document, window and <body> a wheel listener is passive unless it says { passive: false }", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    rt.createApp({ render: () => null }).mount();
    const doc = rt.global.document as unknown as El;
    doc.addEventListener("wheel", (e) => e.preventDefault());
    (rt.global as unknown as El).addEventListener("wheel", (e) => e.preventDefault());
    (doc.body as El).addEventListener("wheel", (e) => e.preventDefault());
    expect(fire(rt, inner, "wheel", 0, 40)).toBe(true);
    doc.addEventListener("wheel", (e) => e.preventDefault(), { passive: false });
    expect(fire(rt, inner, "wheel", 0, 40)).toBe(false);
  });

  test("a cancelled wheel on a <textarea> is advised once: the widget has already scrolled", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { outer } = tree(rt);
    const ta = rt.nodeOps.createElement("textarea") as unknown as El;
    rt.nodeOps.insert(ta as never, outer as never, null);
    ta.addEventListener("wheel", (e) => e.preventDefault());
    expect(fire(rt, ta, "wheel", 0, 40)).toBe(false);
    fire(rt, ta, "wheel", 0, 40);
    expect(rt.advisories().filter((a) => a.includes("already scrolled")).length).toBe(1);
  });

  test("the default applies to wheel and touch only: a document mousedown listener still cancels", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    rt.createApp({ render: () => null }).mount();
    (rt.global.document as unknown as El).addEventListener("mousedown", (e) => e.preventDefault());
    expect(fire(rt, inner, "mousedown", 0, 0, 0)).toBe(false);
  });
});

describe("pointercancel", () => {
  test("its own code; a PointerEvent that bubbles to the window and cannot be cancelled", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    rt.createApp({ render: () => null });
    let seen: Ev | undefined;
    (rt.global as unknown as El).addEventListener("pointercancel", (e) => (seen = e));
    rt.patchProp(rt.el(3), "onPointercancel", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.pointercancel)).toEqual([{ id: 3, code: 44, on: true }]);
    fire(rt, inner, "pointercancel", 7, 8, -1);
    expect(seen?.type).toBe("pointercancel");
    expect([seen!.bubbles, seen!.cancelable]).toEqual([true, false]);
    expect([seen!.pointerType, seen!.pointerId, seen!.isPrimary]).toEqual(["mouse", 1, true]);
    expect([seen!.button, seen!.buttons, seen!.clientX, seen!.clientY]).toEqual([-1, 0, 7, 8]);
    expect(rt.advisories()).toEqual([]);
  });
});

describe("<img> load and error", () => {
  const srcWrites = (rt: Rt, id: number) =>
    rt.host.props
      .filter((p: { id: number; key: string }) => p.id === id && p.key === "src")
      .map((p: { value: unknown }) => p.value);

  const newImg = (rt: Rt) => rt.nodeOps.createElement("img") as unknown as El;

  test("src reaches the host; onload sees the natural size; complete follows the answer", () => {
    const rt = createRuntime();
    const img = newImg(rt);
    const seen: string[] = [];
    img.onload = function (this: El, e: Ev) {
      seen.push(`load:${this === img}:${e.bubbles}:${img.complete}:${img.naturalWidth}x${img.naturalHeight}`);
    };
    expect(img.complete).toBe(true); // no src yet
    img.src = "blob:app://x/1";
    expect(srcWrites(rt, img.__vnId)).toEqual(["blob:app://x/1"]);
    expect(img.src).toBe("blob:app://x/1");
    expect([img.complete, img.naturalWidth]).toEqual([false, 0]);
    fire(rt, img, "load", 640, 480, 0, 0, "blob:app://x/1");
    expect(seen).toEqual(["load:true:false:true:640x480"]);
    expect(img.currentSrc).toBe("blob:app://x/1");
  });

  test("the reka/Argon idiom: @load reads e.target.naturalWidth", () => {
    const rt = createRuntime();
    const img = newImg(rt);
    let aspect = 0;
    rt.patchProp(img as never, "onLoad", null, (e: { target: El }) => {
      aspect = (e.target.naturalWidth as number) / (e.target.naturalHeight as number);
    });
    rt.patchProp(img as never, "src", null, "https://cdn.example/p.png");
    fire(rt, img, "load", 1200, 630, 0, 0, "https://cdn.example/p.png");
    expect(aspect).toBeCloseTo(1200 / 630);
  });

  test("an <img> in the tree is instanceof the ENGINE's HTMLImageElement too", () => {
    class EngineImage {}
    const rt = createRuntime({ globals: { HTMLImageElement: EngineImage } });
    expect(newImg(rt) instanceof EngineImage).toBe(true);
    expect(new EngineImage() instanceof EngineImage).toBe(true);
    expect(rt.nodeOps.createElement("div") instanceof EngineImage).toBe(false);
  });

  test("error: complete and broken, no size; onerror runs; the event is a plain Event", () => {
    const rt = createRuntime();
    const img = newImg(rt);
    let seen: Ev | undefined;
    img.onerror = (e: Ev) => (seen = e);
    img.src = "nope.png";
    fire(rt, img, "error", 0, 0, 0, 0, "nope.png");
    expect(seen?.type).toBe("error");
    expect(seen).not.toHaveProperty("currentTime"); // not a media error's payload
    expect([seen!.bubbles, seen!.cancelable]).toEqual([false, false]);
    expect([img.complete, img.naturalWidth, img.currentSrc]).toEqual([true, 0, ""]);
  });

  test("an answer for a src the element has since dropped is stale, and ignored", () => {
    const rt = createRuntime();
    const img = newImg(rt);
    const seen: string[] = [];
    img.addEventListener("load", () => seen.push("load"));
    img.src = "a.png";
    img.src = "b.png";
    fire(rt, img, "load", 1, 1, 0, 0, "a.png");
    expect(seen).toEqual([]);
    expect(img.complete).toBe(false);
    fire(rt, img, "load", 2, 3, 0, 0, "b.png");
    expect(seen).toEqual(["load"]);
    expect([img.naturalWidth, img.naturalHeight]).toEqual([2, 3]);
  });

  test("load does not bubble; a capture listener above still hears it", () => {
    const rt = createRuntime();
    const { outer } = tree(rt);
    const img = rt.nodeOps.createElement("img") as unknown as El;
    rt.nodeOps.insert(img as never, outer as never, null);
    const seen: string[] = [];
    outer.addEventListener("load", () => seen.push("bubble"));
    outer.addEventListener("load", () => seen.push("capture"), true);
    rt.patchProp(img as never, "src", null, "x.png");
    fire(rt, img, "load", 1, 1, 0, 0, "x.png");
    expect(seen).toEqual(["capture"]);
  });

  test("@load is delivered now, not advised", () => {
    const rt = createRuntime();
    const img = rt.nodeOps.createElement("img");
    rt.patchProp(img, "onLoad", null, () => {});
    expect(rt.advisories()).toEqual([]);
    expect(rt.flagsFor(rt.EV_CODES.load).at(-1)?.on).toBe(true);
  });

  test("decode() settles with the answer", async () => {
    const rt = createRuntime();
    const img = newImg(rt) as El & { decode(): Promise<void> };
    img.src = "ok.png";
    const ok = img.decode();
    fire(rt, img, "load", 4, 4, 0, 0, "ok.png");
    await expect(ok).resolves.toBeUndefined();
    img.src = "bad.png";
    const bad = img.decode();
    fire(rt, img, "error", 0, 0, 0, 0, "bad.png");
    await expect(bad).rejects.toThrow("cannot be decoded");
  });

  test("an empty src is complete, and its answer is an error", () => {
    const rt = createRuntime();
    const img = newImg(rt);
    const seen: string[] = [];
    img.onerror = () => seen.push("error");
    img.src = "";
    expect(img.complete).toBe(true);
    fire(rt, img, "error", 0, 0, 0, 0, "");
    expect(seen).toEqual(["error"]);
  });

  test("HTMLImageElement.prototype carries the accessors, and refuses another element", () => {
    const rt = createRuntime();
    const proto = (rt.global as unknown as { HTMLImageElement: { prototype: object } }).HTMLImageElement.prototype;
    const d = Object.getOwnPropertyDescriptor(proto, "naturalWidth")!;
    const img = rt.nodeOps.createElement("img");
    expect(d.get!.call(img)).toBe(0);
    expect(() => d.get!.call(rt.nodeOps.createElement("div"))).toThrow("Illegal invocation");
    const div = rt.nodeOps.createElement("div") as unknown as El;
    expect([div.naturalWidth, div.complete]).toEqual([undefined, undefined]);
    div.src = "kept";
    expect(div.src).toBe("kept");
    expect(srcWrites(rt, div.__vnId)).toEqual([]);
  });
});

describe("on<event> handler attributes", () => {
  test("one listener, kept in its place while the handler changes; null removes it", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    const seen: string[] = [];
    inner.addEventListener("click", () => seen.push("first"));
    inner.onclick = () => seen.push("handler-a");
    inner.addEventListener("click", () => seen.push("last"));
    fire(rt, inner, "click");
    inner.onclick = () => seen.push("handler-b");
    fire(rt, inner, "click");
    expect(inner.onclick).toBeInstanceOf(Function);
    inner.onclick = null;
    fire(rt, inner, "click");
    expect(inner.onclick).toBe(null);
    expect(seen).toEqual(["first", "handler-a", "last", "first", "handler-b", "last", "first", "last"]);
  });

  test("a non-object is null; returning false cancels", () => {
    const rt = createRuntime({ MouseEvent: FakeMouseEvent });
    const { inner } = tree(rt);
    inner.onmousedown = "alert(1)";
    expect(inner.onmousedown).toBe(null);
    inner.onmousedown = () => false;
    expect(fire(rt, inner, "mousedown")).toBe(false);
  });

  test("on something that is not a mirror element it stays a plain property", () => {
    const rt = createRuntime();
    const proto = Object.getPrototypeOf(rt.nodeOps.createElement("div"));
    const stranger = Object.create(proto) as El;
    const fn = () => {};
    stranger.onchange = fn;
    expect(Object.getOwnPropertyDescriptor(stranger, "onchange")?.value).toBe(fn);
  });
});
