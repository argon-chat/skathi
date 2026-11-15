// The Pointer Lock API's events: the host fires `pointerlockchange` / `pointerlockerror` at node 0 (the document); a
// `mousemove` carries the host's movement in the dispatcher's last two arguments. The API itself is native.

import { describe, expect, test } from "bun:test";
import { createRuntime, type RealmGlobal } from "./runtime-harness";

type Moved = Event & { movementX: number; movementY: number; clientX: number };

function world() {
  const rt = createRuntime();
  rt.createApp({ render: () => null }).mount();
  const global = rt.global as RealmGlobal;
  const doc = global.document;
  const canvas = rt.nodeOps.createElement("canvas");
  rt.nodeOps.insert(canvas, doc.body, null);
  const answer = (name: "pointerlockchange" | "pointerlockerror") =>
    rt.host.dispatcher(0, rt.EV_CODES[name], 0, 0, 0, 0);
  return { rt, doc, canvas, answer };
}

describe("pointerlockchange", () => {
  test("a change no request asked for (Escape, blur) still fires at the document", () => {
    const { doc, answer } = world();
    let changes = 0;
    doc.addEventListener("pointerlockchange", () => changes++);
    expect(answer("pointerlockchange")).toBe(true);
    expect(changes).toBe(1);
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
