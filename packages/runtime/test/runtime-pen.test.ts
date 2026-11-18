// A pen drives the same host mouse events as the mouse, with a packet of readings (sk-tree's pen.rs); these pin the
// PointerEvents built from it, the mouse events that follow, coalesced moves and the boundary events.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

class FakePointerEvent extends Event {
  readonly init: Record<string, unknown>;
  constructor(type: string, init: Record<string, unknown> = {}) {
    super(type, init as EventInit);
    this.init = init;
    for (const k of Object.keys(init))
      if (k !== "coalescedEvents" && k !== "predictedEvents" && k !== "bubbles")
        Object.defineProperty(this, k, { value: init[k], configurable: true });
  }
  getCoalescedEvents(): unknown[] {
    return [...((this.init.coalescedEvents as unknown[] | undefined) ?? [])];
  }
  getPredictedEvents(): unknown[] {
    return [...((this.init.predictedEvents as unknown[] | undefined) ?? [])];
  }
}
class FakeMouseEvent extends FakePointerEvent {}

// The runtime reads `PointerEvent` off the real global, as it would off the engine's.
const real = Object.getOwnPropertyDescriptor(globalThis, "PointerEvent");
beforeAll(() => Object.defineProperty(globalThis, "PointerEvent", { value: FakePointerEvent, configurable: true }));
afterAll(() => {
  if (real) Object.defineProperty(globalThis, "PointerEvent", real);
  else delete (globalThis as { PointerEvent?: unknown }).PointerEvent;
});

type Ev = Event & {
  pointerId: number;
  pointerType: string;
  clientX: number;
  clientY: number;
  offsetX: number;
  button: number;
  buttons: number;
  pressure: number;
  tiltX: number;
  twist: number;
  width: number;
  relatedTarget: unknown;
  getCoalescedEvents(): Ev[];
  getPredictedEvents(): Ev[];
};
type Rt = ReturnType<typeof createRuntime>;

const world = () => {
  const rt = createRuntime({ MouseEvent: FakeMouseEvent });
  const root = rt.nodeOps.createElement("root");
  const canvas = rt.nodeOps.createElement("canvas");
  const other = rt.nodeOps.createElement("div");
  rt.nodeOps.insert(canvas, root, null);
  rt.nodeOps.insert(other, root, null);
  return { rt, root, canvas, other };
};
const listen = (node: MirrorElement, type: string, fn: (e: Ev) => void) =>
  node.addEventListener(type, fn as (ev: Event) => void);

interface Reading {
  x: number;
  y: number;
  pressure?: number;
  tiltX?: number;
  twist?: number;
  time?: number;
}
const ERASER = 1;
const BARREL = 2;
const CONTACT = 4;
const MOUSE = 8;
const packet = (flags: number, ...readings: Reading[]) => [
  flags,
  readings.length,
  ...readings.flatMap((r) => [r.x, r.y, r.pressure ?? 0, 0, r.tiltX ?? 0, 0, r.twist ?? 0, 0, 0, r.time ?? NaN]),
];
const fire = (
  rt: Rt,
  node: MirrorElement,
  name: keyof Rt["EV_CODES"],
  at: Reading,
  button: number,
  pen?: number[],
  key?: string,
) => rt.host.dispatcher(node.__skId, rt.EV_CODES[name], at.x, at.y, button, 0, key, undefined, 0, 0, pen);

describe("pen pointer events", () => {
  test("a stroke: pen fields on the pointer events, mouse events after each", () => {
    const { rt, canvas } = world();
    const seen: unknown[][] = [];
    for (const type of ["pointerdown", "pointermove", "pointerup", "mousedown", "mousemove", "mouseup"])
      listen(canvas, type, (e) => seen.push([e.type, e.pointerType, e.pointerId, e.button, e.buttons, e.pressure]));
    const at = { x: 10, y: 20, pressure: 0.25, tiltX: 30, twist: 90 };
    fire(rt, canvas, "mousedown", at, 0, packet(CONTACT, at));
    fire(rt, canvas, "mousemove", at, 0, packet(CONTACT, { ...at, pressure: 0.75 }));
    fire(rt, canvas, "mouseup", at, 0, packet(0, { ...at, pressure: 0 }));
    expect(seen).toEqual([
      ["pointerdown", "pen", 2, 0, 1, 0.25],
      ["mousedown", undefined, undefined, 0, 1, undefined],
      ["pointermove", "pen", 2, -1, 1, 0.75],
      ["mousemove", undefined, undefined, 0, 1, undefined],
      ["pointerup", "pen", 2, 0, 0, 0],
      ["mouseup", undefined, undefined, 0, 0, undefined],
    ]);
  });

  test("tilt and twist come through; a hovering pen has no pressure and no buttons", () => {
    const { rt, canvas } = world();
    const seen: Ev[] = [];
    listen(canvas, "pointermove", (e) => seen.push(e));
    fire(rt, canvas, "mousemove", { x: 1, y: 2 }, -1, packet(0, { x: 1, y: 2, pressure: 0.4, tiltX: -45, twist: 10 }));
    expect([seen[0]?.pointerType, seen[0]?.buttons, seen[0]?.pressure, seen[0]?.tiltX, seen[0]?.twist]).toEqual([
      "pen",
      0,
      0,
      -45,
      10,
    ]);
    expect(seen[0]?.width).toBe(1);
  });

  test("the eraser is button 5, buttons 32; the barrel is button 2", () => {
    const { rt, canvas } = world();
    const seen: number[][] = [];
    listen(canvas, "pointerdown", (e) => seen.push([e.button, e.buttons]));
    listen(canvas, "pointermove", (e) => seen.push([e.button, e.buttons]));
    const at = { x: 0, y: 0, pressure: 0.5 };
    fire(rt, canvas, "mousedown", at, 0, packet(ERASER | CONTACT, at));
    fire(rt, canvas, "mousemove", at, 0, packet(ERASER | CONTACT, at));
    fire(rt, canvas, "mouseup", at, 0, packet(ERASER, at));
    fire(rt, canvas, "mousedown", at, 2, packet(BARREL | CONTACT, at));
    expect(seen).toEqual([
      [5, 32],
      [-1, 32],
      [2, 2],
    ]);
  });

  test("a move's coalesced events are its readings, oldest first, each with its own movement", () => {
    const { rt, canvas } = world();
    let coalesced: Ev[] = [];
    let target: unknown;
    listen(canvas, "pointermove", (e) => {
      coalesced = e.getCoalescedEvents();
      target = e.target;
    });
    const readings = [
      { x: 1, y: 0, pressure: 0.1 },
      { x: 3, y: 0, pressure: 0.2 },
      { x: 6, y: 0, pressure: 0.3 },
    ];
    rt.host.dispatcher(
      canvas.__skId,
      rt.EV_CODES.mousemove,
      6,
      0,
      0,
      0,
      undefined,
      undefined,
      6,
      0,
      packet(CONTACT, ...readings),
    );
    expect(
      coalesced.map((e) => [e.type, e.clientX, e.pressure, (e as unknown as { movementX: number }).movementX]),
    ).toEqual([
      ["pointermove", 1, 0.1, 1],
      ["pointermove", 3, 0.2, 2],
      ["pointermove", 6, 0.3, 3],
    ]);
    expect(target).toBe(canvas);
  });

  test("coalesced events take their readings' times; predicted events follow the move with theirs", () => {
    const { rt, canvas } = world();
    let seen: Ev | undefined;
    listen(canvas, "pointermove", (e) => (seen = e));
    const pen = [
      ...packet(CONTACT, { x: 2, y: 0, pressure: 0.2, time: 100 }, { x: 4, y: 0, pressure: 0.4, time: 104 }),
      ...[6, 1, 108, 8, 2, 112],
    ];
    rt.host.dispatcher(canvas.__skId, rt.EV_CODES.mousemove, 4, 0, 0, 0, undefined, undefined, 2, 0, pen);
    expect(seen?.getCoalescedEvents().map((e) => e.timeStamp)).toEqual([100, 104]);
    const movement = (e: Ev) => [
      (e as unknown as { movementX: number }).movementX,
      (e as unknown as { movementY: number }).movementY,
    ];
    expect(
      seen?.getPredictedEvents().map((e) => [e.type, e.clientX, e.clientY, e.timeStamp, e.pressure, ...movement(e)]),
    ).toEqual([
      ["pointermove", 6, 1, 108, 0.4, 2, 1],
      ["pointermove", 8, 2, 112, 0.4, 2, 1],
    ]);
  });

  test("a mouse move coalesces its positions too, and a lone move lists itself", () => {
    const { rt, canvas } = world();
    const lists: unknown[][] = [];
    listen(canvas, "pointermove", (e) =>
      lists.push(e.getCoalescedEvents().map((c) => [c.pointerType, c.clientX, c.buttons])),
    );
    fire(rt, canvas, "mousemove", { x: 9, y: 0 }, 0, packet(MOUSE, { x: 4, y: 0 }, { x: 9, y: 0 }));
    fire(rt, canvas, "mousemove", { x: 12, y: 0 }, 0);
    expect(lists).toEqual([
      [
        ["mouse", 4, 1],
        ["mouse", 9, 1],
      ],
      [["mouse", 12, 1]],
    ]);
  });

  test("cancelling a pen's pointerdown suppresses its mouse events until it lifts; click still comes", () => {
    const { rt, canvas } = world();
    const seen: string[] = [];
    listen(canvas, "pointerdown", (e) => e.preventDefault());
    for (const type of ["pointermove", "pointerup", "mousedown", "mousemove", "mouseup", "click"])
      listen(canvas, type, (e) => seen.push(e.type));
    const at = { x: 0, y: 0, pressure: 0.5 };
    expect(fire(rt, canvas, "mousedown", at, 0, packet(CONTACT, at))).toBe(false);
    fire(rt, canvas, "mousemove", at, 0, packet(CONTACT, at));
    fire(rt, canvas, "mouseup", at, 0, packet(0, at));
    fire(rt, canvas, "click", at, 0, packet(0, at));
    fire(rt, canvas, "mousemove", at, -1, packet(0, at));
    expect(seen).toEqual(["pointermove", "pointerup", "click", "pointermove", "mousemove"]);
  });

  test("setPointerCapture(e.pointerId) captures the pen", () => {
    const { rt, root, canvas } = world();
    const seen: unknown[] = [];
    listen(canvas, "pointerdown", (e) => canvas.setPointerCapture(e.pointerId));
    listen(canvas, "pointermove", (e) => seen.push(e.target === canvas));
    const at = { x: 0, y: 0, pressure: 0.5 };
    fire(rt, canvas, "mousedown", at, 0, packet(CONTACT, at));
    expect(canvas.hasPointerCapture(2)).toBe(true);
    fire(rt, root, "mousemove", { x: 500, y: 0 }, 0, packet(CONTACT, { x: 500, y: 0 }));
    fire(rt, root, "mouseup", { x: 500, y: 0 }, 0, packet(0, { x: 500, y: 0 }));
    expect(canvas.hasPointerCapture(2)).toBe(false);
    expect(seen).toEqual([true]);
    expect(rt.host.pointerCaptureCalls.map((c) => [c.pointerId, c.capture])).toEqual([
      [2, true],
      [2, false],
    ]);
    expect(() => canvas.setPointerCapture(3)).toThrow();
  });
});

describe("boundary events", () => {
  test("pointerover/pointerout bubble, carry the pen, and name the other side", () => {
    const { rt, root, canvas, other } = world();
    const seen: unknown[][] = [];
    for (const type of ["pointerover", "pointerout", "mouseover", "mouseout"])
      listen(root, type, (e) =>
        seen.push([e.type, e.target === canvas ? "canvas" : "other", e.relatedTarget === other, e.pointerType]),
      );
    const at = { x: 0, y: 0 };
    fire(rt, canvas, "mouseover", at, -1, packet(0, at), String(other.__skId));
    fire(rt, canvas, "mouseout", at, -1, packet(0, at), String(other.__skId));
    expect(seen).toEqual([
      ["pointerover", "canvas", true, "pen"],
      ["mouseover", "canvas", true, undefined],
      ["pointerout", "canvas", true, "pen"],
      ["mouseout", "canvas", true, undefined],
    ]);
  });
});

describe("offsetX", () => {
  test("is measured from the target's box", () => {
    const { rt, canvas } = world();
    (canvas as unknown as { getBoundingClientRect: () => object }).getBoundingClientRect = () => ({
      left: 100,
      top: 50,
    });
    let offset: number[] = [];
    listen(canvas, "pointerdown", (e) => (offset = [e.offsetX, (e as unknown as { offsetY: number }).offsetY]));
    fire(rt, canvas, "mousedown", { x: 130, y: 70 }, 0);
    expect(offset).toEqual([30, 20]);
  });
});
