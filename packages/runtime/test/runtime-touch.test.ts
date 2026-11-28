// Touch contacts drive the host's mouse codes with a touch packet (sk-tree's touch.rs): each contact is a pointer of its
// own, only the primary one fires mouse events, and each is captured implicitly to where it went down.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

class FakePointerEvent extends Event {
  constructor(type: string, init: Record<string, unknown> = {}) {
    super(type, init as EventInit);
    for (const k of Object.keys(init))
      if (k !== "coalescedEvents" && k !== "predictedEvents" && k !== "bubbles" && k !== "cancelable")
        Object.defineProperty(this, k, { value: init[k], configurable: true });
  }
}
class FakeMouseEvent extends FakePointerEvent {}

const real = Object.getOwnPropertyDescriptor(globalThis, "PointerEvent");
beforeAll(() => Object.defineProperty(globalThis, "PointerEvent", { value: FakePointerEvent, configurable: true }));
afterAll(() => {
  if (real) Object.defineProperty(globalThis, "PointerEvent", real);
  else delete (globalThis as { PointerEvent?: unknown }).PointerEvent;
});

type Ev = Event & {
  pointerId: number;
  pointerType: string;
  isPrimary: boolean;
  buttons: number;
  pressure: number;
  width: number;
  height: number;
};
type Rt = ReturnType<typeof createRuntime>;

const world = () => {
  const rt = createRuntime({ MouseEvent: FakeMouseEvent });
  const root = rt.nodeOps.createElement("root");
  const a = rt.nodeOps.createElement("div");
  const b = rt.nodeOps.createElement("div");
  rt.nodeOps.insert(a, root, null);
  rt.nodeOps.insert(b, root, null);
  return { rt, root, a, b };
};
const listen = (node: MirrorElement, type: string, fn: (e: Ev) => void) =>
  node.addEventListener(type, fn as (ev: Event) => void);

const TOUCH = 32;
const PRIMARY = 64;
interface Contact {
  id: number;
  primary: boolean;
  x?: number;
  pressure?: number;
  width?: number;
  height?: number;
}
const packet = (c: Contact) => [
  TOUCH + (c.primary ? PRIMARY : 0) + c.id * 256,
  1,
  c.x ?? 0,
  0,
  c.pressure ?? 0.5,
  0,
  0,
  0,
  0,
  c.width ?? 0,
  c.height ?? 0,
  NaN,
];
const fire = (rt: Rt, node: MirrorElement, name: keyof Rt["EV_CODES"], c: Contact, button: number) =>
  rt.host.dispatcher(node.__skId, rt.EV_CODES[name], c.x ?? 0, 0, button, 0, undefined, undefined, 0, 0, packet(c));

const POINTER = ["pointerdown", "pointermove", "pointerup", "pointercancel"];
const MOUSE = ["mousedown", "mousemove", "mouseup", "click"];

describe("touch pointers", () => {
  test("two contacts: their own ids, one primary, mouse events for the primary only", () => {
    const { rt, a, b } = world();
    const seen: string[] = [];
    for (const node of [a, b])
      for (const type of [...POINTER, ...MOUSE])
        listen(node, type, (e) =>
          seen.push(e.pointerType ? `${e.type}:${e.pointerType}:${e.pointerId}:${e.isPrimary}` : e.type),
        );
    const first = { id: 3, primary: true };
    const second = { id: 4, primary: false };
    fire(rt, a, "mousedown", first, 0);
    fire(rt, b, "mousedown", second, 0);
    fire(rt, b, "mousemove", second, 0);
    fire(rt, a, "mousemove", first, 0);
    fire(rt, b, "mouseup", second, 0);
    fire(rt, a, "mouseup", first, 0);
    fire(rt, a, "click", first, 0);
    expect(seen).toEqual([
      "pointerdown:touch:3:true",
      "mousedown",
      "pointerdown:touch:4:false",
      "pointermove:touch:4:false",
      "pointermove:touch:3:true",
      "mousemove",
      "pointerup:touch:4:false",
      "pointerup:touch:3:true",
      "mouseup",
      "click",
    ]);
  });

  test("ten contacts at once keep ten captures, each released by its own lift", () => {
    const { rt, a, b } = world();
    const contacts = Array.from({ length: 10 }, (_, i) => ({ id: 3 + i, primary: i === 0 }));
    contacts.forEach((c, i) => fire(rt, i % 2 ? b : a, "mousedown", c, 0));
    expect(contacts.map((c, i) => (i % 2 ? b : a).hasPointerCapture(c.id))).toEqual(Array(10).fill(true));
    fire(rt, b, "mouseup", contacts[3]!, 0);
    expect(b.hasPointerCapture(6)).toBe(false);
    expect(a.hasPointerCapture(3)).toBe(true);
    expect(b.hasPointerCapture(4)).toBe(true);
  });

  test("width, height and pressure come from the contact; no geometry reads 1 by 1", () => {
    const { rt, a } = world();
    const seen: number[][] = [];
    for (const type of ["pointerdown", "pointerup"])
      listen(a, type, (e) => seen.push([e.width, e.height, e.pressure, e.buttons]));
    fire(rt, a, "mousedown", { id: 3, primary: true, width: 12, height: 18, pressure: 0.3 }, 0);
    fire(rt, a, "mouseup", { id: 3, primary: true }, 0);
    expect(seen).toEqual([
      [12, 18, 0.3, 1],
      [1, 1, 0, 0],
    ]);
  });

  test("implicit capture: gotpointercapture after pointerdown, lostpointercapture after pointerup", () => {
    const { rt, a } = world();
    const seen: string[] = [];
    for (const type of ["pointerdown", "gotpointercapture", "pointerup", "lostpointercapture"])
      listen(a, type, (e) => seen.push(`${e.type}:${e.pointerId}:${a.hasPointerCapture(e.pointerId)}`));
    const c = { id: 5, primary: false };
    fire(rt, a, "mousedown", c, 0);
    fire(rt, a, "mouseup", c, 0);
    expect(seen).toEqual([
      "pointerdown:5:true",
      "gotpointercapture:5:true",
      "pointerup:5:true",
      "lostpointercapture:5:false",
    ]);
    expect(rt.host.pointerCaptureCalls.map((x) => [x.pointerId, x.capture])).toEqual([
      [5, true],
      [5, false],
    ]);
  });

  test("releasePointerCapture in pointerdown leaves the contact uncaptured, with no capture events", () => {
    const { rt, a } = world();
    const seen: string[] = [];
    listen(a, "pointerdown", (e) => a.releasePointerCapture(e.pointerId));
    for (const type of ["gotpointercapture", "lostpointercapture"]) listen(a, type, (e) => seen.push(e.type));
    fire(rt, a, "mousedown", { id: 3, primary: true }, 0);
    expect(a.hasPointerCapture(3)).toBe(false);
    expect(seen).toEqual([]);
  });

  test("setPointerCapture moves a contact's capture; a contact that is not down is not a pointer", () => {
    const { rt, a, b } = world();
    const seen: string[] = [];
    listen(a, "pointerdown", (e) => b.setPointerCapture(e.pointerId));
    for (const node of [a, b])
      for (const type of ["gotpointercapture", "lostpointercapture"])
        listen(node, type, (e) => seen.push(`${e.type}@${node === a ? "a" : "b"}`));
    fire(rt, a, "mousedown", { id: 7, primary: false }, 0);
    expect(b.hasPointerCapture(7)).toBe(true);
    expect(seen).toEqual(["gotpointercapture@b"]);
    expect(() => a.setPointerCapture(99)).toThrow();
  });

  test("a cancelled pointerdown keeps the primary's mouse events away until it lifts; click still comes", () => {
    const { rt, a } = world();
    const seen: string[] = [];
    listen(a, "pointerdown", (e) => e.preventDefault());
    for (const type of ["pointermove", "pointerup", ...MOUSE, "mouseover"]) listen(a, type, (e) => seen.push(e.type));
    const c = { id: 3, primary: true };
    fire(rt, a, "mousedown", c, 0);
    fire(rt, a, "mousemove", c, 0);
    fire(rt, a, "mouseup", c, 0);
    fire(rt, a, "click", c, 0);
    fire(rt, a, "mouseover", c, -1);
    expect(seen).toEqual(["pointermove", "pointerup", "click", "mouseover"]);
  });

  test("pointercancel ends a contact and releases its capture", () => {
    const { rt, a } = world();
    const seen: string[] = [];
    for (const type of ["pointercancel", "lostpointercapture"]) listen(a, type, (e) => seen.push(e.type));
    const c = { id: 9, primary: false };
    fire(rt, a, "mousedown", c, 0);
    fire(rt, a, "pointercancel", c, -1);
    expect(seen).toEqual(["pointercancel", "lostpointercapture"]);
    expect(a.hasPointerCapture(9)).toBe(false);
  });

  test("a finger leaves the mouse's capture alone", () => {
    const { rt, a, b } = world();
    listen(a, "pointerdown", (e) => {
      if (e.pointerType === "mouse") a.setPointerCapture(e.pointerId);
    });
    rt.host.dispatcher(a.__skId, rt.EV_CODES.mousedown, 0, 0, 0, 0, undefined, undefined, 0, 0, undefined);
    fire(rt, b, "mousedown", { id: 3, primary: true }, 0);
    fire(rt, b, "mouseup", { id: 3, primary: true }, 0);
    expect(a.hasPointerCapture(1)).toBe(true);
  });
});
