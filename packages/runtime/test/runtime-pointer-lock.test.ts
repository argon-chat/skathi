// The Pointer Lock API's JS half: requests go to the host (`host.pointerLock`), whose answers arrive as
// `pointerlockchange` / `pointerlockerror` at node 0 carrying the request they settle; a `mousemove` carries the
// host's movement in the dispatcher's last two arguments.

import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement, type RealmDocument, type RealmGlobal } from "./runtime-harness";

interface LockDocument extends RealmDocument {
  readonly pointerLockElement: unknown;
  exitPointerLock(): void;
}
type LockElement = MirrorElement & { requestPointerLock(options?: { unadjustedMovement?: boolean }): Promise<void> };
type Moved = Event & { movementX: number; movementY: number; clientX: number };

function world() {
  const rt = createRuntime();
  rt.createApp({ render: () => null }).mount();
  const global = rt.global as RealmGlobal & { addEventListener(type: string, fn: () => void): void };
  const doc = global.document as LockDocument;
  const canvas = rt.nodeOps.createElement("canvas") as LockElement;
  rt.nodeOps.insert(canvas, doc.body, null);
  const answer = (name: "pointerlockchange" | "pointerlockerror", request: number, key?: string) =>
    rt.host.dispatcher(0, rt.EV_CODES[name], request, 0, 0, 0, key);
  return { rt, global, doc, canvas, answer };
}

describe("requestPointerLock", () => {
  test("asks the host and resolves on the change that settles it, after document listeners", async () => {
    const { rt, global, doc, canvas, answer } = world();
    const seen: string[] = [];
    doc.addEventListener("pointerlockchange", (e) => seen.push(`document:${String(e.bubbles)}`));
    global.addEventListener("pointerlockchange", () => seen.push("window"));
    const locked = canvas.requestPointerLock({ unadjustedMovement: true }).then(() => seen.push("resolved"));
    expect(rt.host.pointerLock.requests).toEqual([{ id: canvas.__skId, unadjusted: true }]);

    rt.host.pointerLock.element = canvas.__skId;
    answer("pointerlockchange", 1);
    expect(doc.pointerLockElement).toBe(canvas);
    await locked;
    expect(seen).toEqual(["document:true", "window", "resolved"]);
  });

  test("rejects with the host's DOMException and fires pointerlockerror", async () => {
    const { doc, canvas, answer } = world();
    let errors = 0;
    doc.addEventListener("pointerlockerror", () => errors++);
    const first = canvas.requestPointerLock();
    const second = canvas.requestPointerLock();
    answer("pointerlockerror", 2, "WrongDocumentError: Pointer lock needs the window to be focused.");
    const e = await second.catch((err: unknown) => err as DOMException);
    expect(e).toBeInstanceOf(DOMException);
    expect(e && (e as DOMException).name).toBe("WrongDocumentError");
    expect(e && (e as DOMException).message).toBe("Pointer lock needs the window to be focused.");
    expect(errors).toBe(1);

    answer("pointerlockchange", 1);
    await first;
    expect(doc.pointerLockElement).toBeNull();
  });

  test("a change no request asked for (Escape, blur) settles nothing but still fires", () => {
    const { doc, answer } = world();
    let changes = 0;
    doc.addEventListener("pointerlockchange", () => changes++);
    expect(answer("pointerlockchange", 0)).toBe(true);
    expect(changes).toBe(1);
  });

  test("exitPointerLock goes to the host", () => {
    const { rt, doc } = world();
    doc.exitPointerLock();
    expect(rt.host.pointerLock.exits).toBe(1);
  });
});

describe("movement", () => {
  test("mousemove and pointermove carry the host's movement; other mouse events carry none", () => {
    const { rt, canvas } = world();
    const moves: Moved[] = [];
    rt.patchProp(canvas, "onMousemove", null, (e: Moved) => moves.push(e));
    rt.patchProp(canvas, "onPointermove", null, (e: Moved) => moves.push(e));
    rt.patchProp(canvas, "onMousedown", null, (e: Moved) => moves.push(e));
    rt.host.dispatcher(canvas.__skId, rt.EV_CODES.mousemove, 40, 50, -1, 0, undefined, undefined, 12, -7);
    rt.host.dispatcher(canvas.__skId, rt.EV_CODES.mousedown, 40, 50, 0, 0, undefined, undefined, 3, 3);
    expect(moves.map((e) => [e.type, e.clientX, e.movementX ?? 0, e.movementY ?? 0])).toEqual([
      ["pointermove", 40, 12, -7],
      ["mousemove", 40, 12, -7],
      ["mousedown", 40, 0, 0],
    ]);
  });
});
