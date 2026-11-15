// HTML drag and drop within the app (runtime/dnd.ts `dndDispatch`): the runtime
// runs the gesture off the host's mouse events, since gpui drags no DOM
// elements. A press on a `draggable="true"` element, a move past 4 px, and the
// drag events fire in a browser's order with one DataTransfer.

import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

type N = MirrorElement;
type DragEv = {
  type: string;
  target: N;
  relatedTarget: N | null;
  clientX: number;
  clientY: number;
  cancelable: boolean;
  bubbles: boolean;
  dataTransfer: { dropEffect: string; effectAllowed: string; setData?(t: string, v: string): void };
  preventDefault(): void;
};

const DRAG_EVENTS = ["dragstart", "drag", "dragenter", "dragover", "dragleave", "drop", "dragend"];

/** `<ul><li draggable><span/></li><li id=b/><li id=c/></ul>` under the root. */
function world() {
  const rt = createRuntime();
  rt.createApp({ render: () => null }).mount();
  const body = rt.global.document.body;
  const el = (tag: string, parent: N) => {
    const e = rt.nodeOps.createElement(tag);
    rt.nodeOps.insert(e, parent, null);
    return e;
  };
  const list = el("ul", body);
  const a = el("li", list);
  const label = el("span", a);
  const b = el("li", list);
  const c = el("li", list);
  rt.patchProp(a, "draggable", null, true);
  const log: string[] = [];
  const events: DragEv[] = [];
  const name = (n: N) =>
    n === a ? "a" : n === b ? "b" : n === c ? "c" : n === list ? "list" : n === label ? "label" : "?";
  for (const ev of DRAG_EVENTS) {
    rt.patchProp(list, `on${ev.charAt(0).toUpperCase()}${ev.slice(1)}`, null, (e: DragEv) => {
      log.push(`${e.type}@${name(e.target)}`);
      events.push(e);
    });
  }
  const { mousedown, mousemove, mouseup, click, keydown } = rt.EV_CODES;
  const send = (id: number, code: number, x: number, y: number, c = 0, key?: string) =>
    rt.host.dispatcher(id, code, x, y, c, 0, key);
  const press = (n: N, x = 10, y = 10) => send(n.__skId, mousedown, x, y, 0);
  const move = (n: N, x: number, y: number, held = true) => send(n.__skId, mousemove, x, y, held ? 0 : -1);
  const release = (n: N, x: number, y: number) => send(n.__skId, mouseup, x, y, 0);
  const clickAt = (n: N) => send(n.__skId, click, 0, 0, 0);
  const escape = (n: N) => send(n.__skId, keydown, 0, 0, 0, "Escape");
  return { rt, body, list, a, label, b, c, log, events, press, move, release, clickAt, escape };
}

describe("arming", () => {
  test("a draggable element arms the root's press and selectstart; a pending press arms its moves", () => {
    const { rt, label, press, move, release } = world();
    const root = rt.global.document.body as N;
    const on = (code: number) =>
      rt
        .flagsFor(code)
        .filter((l) => l.id === root.__skId)
        .at(-1)?.on ?? false;
    expect(on(rt.EV_CODES.mousedown)).toBe(true);
    expect(on(rt.EV_CODES.selectstart)).toBe(true);
    expect(on(rt.EV_CODES.mousemove)).toBe(false);
    press(label);
    expect(on(rt.EV_CODES.mousemove)).toBe(true);
    expect(on(rt.EV_CODES.mouseup)).toBe(true);
    move(label, 30, 10);
    expect(on(rt.EV_CODES.keydown)).toBe(true);
    release(label, 30, 10);
    expect(on(rt.EV_CODES.mousemove)).toBe(false);
    expect(on(rt.EV_CODES.mouseup)).toBe(false);
    expect(on(rt.EV_CODES.keydown)).toBe(false);
  });
});

describe("the gesture", () => {
  test("a drop: dragstart, enter/over, leave as the target changes, drop, dragend", () => {
    const { a, b, c, label, log, events, press, move, release } = world();
    const list = a.parentNode!;
    // b and c accept: their dragover is cancelled.
    const accept = (e: DragEv) => {
      if (e.type === "dragover" && (e.target === b || e.target === c)) {
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
      }
      if (e.type === "dragstart") {
        e.dataTransfer.effectAllowed = "move";
      }
    };
    (list as unknown as { addEventListener(n: string, f: (e: DragEv) => void): void }).addEventListener(
      "dragover",
      accept,
    );
    (list as unknown as { addEventListener(n: string, f: (e: DragEv) => void): void }).addEventListener(
      "dragstart",
      accept,
    );
    press(label, 10, 10);
    move(label, 12, 12); // inside the threshold: still a press
    expect(log).toEqual([]);
    move(b, 10, 30);
    move(c, 10, 50);
    release(c, 10, 50);
    expect(log).toEqual([
      "dragstart@a",
      "drag@a",
      "dragenter@b",
      "dragover@b",
      "drag@a",
      "dragenter@c",
      "dragleave@b",
      "dragover@c",
      "drop@c",
      "dragend@a",
    ]);
    const dt = events[0]!.dataTransfer;
    expect(events.every((e) => e.dataTransfer === dt)).toBe(true);
    const enterC = events.find((e) => e.type === "dragenter" && e.target === c)!;
    const leaveB = events.find((e) => e.type === "dragleave")!;
    expect(enterC.relatedTarget).toBe(b);
    expect(leaveB.relatedTarget).toBe(c);
    const drop = events.find((e) => e.type === "drop")!;
    expect([drop.clientX, drop.clientY, drop.cancelable, drop.bubbles]).toEqual([10, 50, true, true]);
    expect(events.find((e) => e.type === "dragleave")!.cancelable).toBe(false);
  });

  test("a target that does not cancel dragover takes no drop: dragleave, then dragend with no effect", () => {
    const { b, label, log, events, press, move, release } = world();
    press(label);
    move(b, 10, 40);
    release(b, 10, 40);
    expect(log).toEqual(["dragstart@a", "drag@a", "dragenter@b", "dragover@b", "dragleave@b", "dragend@a"]);
    expect(events.at(-1)!.dataTransfer.dropEffect).toBe("none");
  });

  test("the release's click is swallowed, and only that one", () => {
    const { rt, list, b, label, press, move, release, clickAt } = world();
    const clicks: string[] = [];
    rt.patchProp(list, "onClick", null, () => clicks.push("click"));
    press(label);
    move(b, 10, 40);
    release(b, 10, 40);
    expect(clickAt(list)).toBe(false);
    expect(clicks).toEqual([]);
    press(b);
    release(b, 10, 10);
    clickAt(b);
    expect(clicks).toEqual(["click"]);
  });

  test("moves during a drag are not mouse events", () => {
    const { rt, list, b, label, press, move } = world();
    const moves: number[] = [];
    rt.patchProp(list, "onMousemove", null, (e: { clientX: number }) => moves.push(e.clientX));
    move(b, 1, 1); // no press: an ordinary move
    press(label);
    move(b, 40, 40);
    move(b, 50, 50);
    expect(moves).toEqual([1]);
  });

  test("a press that is released in place is a click, and drags nothing", () => {
    const { rt, a, label, log, press, release, clickAt } = world();
    const clicks: string[] = [];
    rt.patchProp(a, "onClick", null, () => clicks.push("click"));
    press(label);
    release(label, 10, 10);
    clickAt(label);
    expect(log).toEqual([]);
    expect(clicks).toEqual(["click"]);
  });

  test("a cancelled mousedown or dragstart starts no drag", () => {
    const { rt, a, b, label, log, press, move, release } = world();
    rt.patchProp(a, "onMousedown", null, (e: DragEv) => e.preventDefault());
    press(label);
    move(b, 10, 40);
    release(b, 10, 40);
    expect(log).toEqual([]);
    rt.patchProp(a, "onMousedown", (e: DragEv) => e.preventDefault(), null);
    rt.patchProp(a, "onDragstart", null, (e: DragEv) => e.preventDefault());
    press(label);
    move(b, 10, 40);
    move(b, 10, 60);
    release(b, 10, 60);
    expect(log).toEqual(["dragstart@a"]);
  });

  test("Escape cancels; the release after it is not heard", () => {
    const { rt, list, b, label, log, press, move, release, escape, clickAt } = world();
    const ups: string[] = [];
    rt.patchProp(list, "onMouseup", null, () => ups.push("up"));
    rt.patchProp(list, "onClick", null, () => ups.push("click"));
    press(label);
    move(b, 10, 40);
    expect(escape(b)).toBe(false);
    move(b, 10, 60);
    release(b, 10, 60);
    clickAt(list);
    expect(log).toEqual(["dragstart@a", "drag@a", "dragenter@b", "dragover@b", "dragleave@b", "dragend@a"]);
    expect(ups).toEqual([]);
  });

  test("a release that never arrived ends the drag at the next unpressed move", () => {
    const { b, c, label, log, press, move } = world();
    press(label);
    move(b, 10, 40);
    move(c, 10, 60, false);
    expect(log).toEqual(["dragstart@a", "drag@a", "dragenter@b", "dragover@b", "dragleave@b", "dragend@a"]);
  });

  test("a press in a draggable selects no text; elsewhere selectstart is the page's", () => {
    const { rt, b, label } = world();
    expect(rt.host.dispatcher(label.__skId, rt.EV_CODES.selectstart, 0, 0, 0, 0, undefined)).toBe(false);
    expect(rt.host.dispatcher(b.__skId, rt.EV_CODES.selectstart, 0, 0, 0, 0, undefined)).toBe(true);
  });

  test("dragover's dropEffect starts from effectAllowed", () => {
    const { a, b, label, events, press, move } = world();
    const list = a.parentNode as unknown as { addEventListener(n: string, f: (e: DragEv) => void): void };
    list.addEventListener("dragstart", (e) => (e.dataTransfer.effectAllowed = "copyMove"));
    const seen: string[] = [];
    list.addEventListener("dragover", (e) => seen.push(e.dataTransfer.dropEffect));
    press(label);
    move(b, 10, 40);
    expect(seen).toEqual(["copy"]);
    expect(events[0]!.dataTransfer.effectAllowed).toBe("copyMove");
  });
});
