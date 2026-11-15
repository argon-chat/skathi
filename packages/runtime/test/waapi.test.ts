// Web Animations' object model (src/waapi.ts): `Element.animate()`, the `Animation` state machine, promises and events.
// The host's painting half is a fake that records the effect and the clock.

import { describe, expect, test } from "bun:test";
import { installWaapi } from "../src/waapi";
import { createReferenceWaapiCore, referenceEffect, referenceValues } from "./fixtures/waapi-core-reference";

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any

function world() {
  let now = 1000;
  const frames: ((ts: number) => void)[] = [];
  const stamps: ((ts: number) => void)[] = [];
  let timers: { at: number; cb: () => void; id: number }[] = [];
  let timerId = 0;
  const calls: Any[] = [];
  const warnings: string[] = [];
  const styles: [string, string][] = [];
  let values: string | null = null;
  const host = {
    setEffect(id: number, node: number, frames: Any[], timing: Any) {
      const projected = referenceEffect(frames);
      calls.push({ op: "set", id, node, frames: projected.frames, timing: { ...timing }, easing: timing.easing });
      return projected.notes;
    },
    clock(id: number, start: number, hold: number, rate: number) {
      calls.push({ op: "clock", id, start, hold, rate });
    },
    drop(id: number) {
      calls.push({ op: "drop", id });
    },
    values(id: number) {
      calls.push({ op: "values", id });
      return referenceValues(values);
    },
  };
  const HTMLProto: Any = {};
  const SVGProto: Any = {};
  const ElementProto: Any = {};
  const globals: Record<string, unknown> = {};
  const doc: Record<string, unknown> = {};
  const api = installWaapi({
    host,
    core: createReferenceWaapiCore(),
    protos: [HTMLProto, SVGProto],
    nodeOf: (t: Any) => t.__skId ?? 0,
    setStyle: (_t, name, value) => styles.push([name, value]),
    contains: (root: Any, node: Any) => {
      for (let n = node; n; n = n.parentNode) if (n === root) return true;
      return false;
    },
    warn: (key, message) => {
      if (!warnings.includes(key)) warnings.push(key);
      void message;
    },
    now: () => now,
    frame: (cb) => frames.push(cb),
    onFrame: (stamp) => stamps.push(stamp),
    setTimer: (cb, ms) => {
      timers.push({ at: now + ms, cb, id: ++timerId });
      return timerId;
    },
    clearTimer: (h) => {
      timers = timers.filter((t) => t.id !== h);
    },
    defineGlobal: (name, value) => (globals[name] = value),
    doc,
    EventTarget,
    Event,
    DOMException: DOMException as Any,
  });
  const el = (id: number): Any => Object.assign(Object.create(HTMLProto), { __skId: id, parentNode: null });
  const frame = async (ms = 16) => {
    now += ms;
    const due = timers.filter((t) => t.at <= now).sort((a, b) => a.at - b.at);
    timers = timers.filter((t) => t.at > now);
    for (const t of due) t.cb();
    const batch = frames.splice(0);
    for (const cb of batch) {
      for (const stamp of stamps) stamp(now);
      cb(now);
    }
    await flush();
  };
  const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };
  return {
    api,
    calls,
    warnings,
    styles,
    globals,
    doc,
    el,
    frame,
    flush,
    raf: (cb: (ts: number) => void) => frames.push(cb),
    tick: (ms: number) => void (now += ms),
    HTMLProto,
    SVGProto,
    ElementProto,
    get now() {
      return now;
    },
    set values(v: string | null) {
      values = v;
    },
    last: (op: string) => [...calls].reverse().find((c) => c.op === op),
  };
}

describe("Element.animate()", () => {
  test("lives on the HTML and SVG prototypes only, and says so once", () => {
    const w = world();
    expect(typeof w.HTMLProto.animate).toBe("function");
    expect(typeof w.SVGProto.animate).toBe("function");
    expect(typeof w.HTMLProto.getAnimations).toBe("function");
    // Element.prototype stays bare: motion's `Object.hasOwnProperty.call(Element.prototype, "animate")` is false.
    expect(Object.prototype.hasOwnProperty.call(w.ElementProto, "animate")).toBe(false);
    expect(w.warnings).toEqual([]);
    w.el(1).animate({ opacity: [0, 1] }, 100);
    w.el(2).animate({ opacity: [0, 1] }, 100);
    expect(w.warnings).toEqual(["waapi:placement"]);
    for (const name of [
      "Animation",
      "KeyframeEffect",
      "AnimationEffect",
      "DocumentTimeline",
      "AnimationTimeline",
      "AnimationPlaybackEvent",
    ]) {
      expect(typeof w.globals[name]).toBe("function");
    }
    expect((w.doc.timeline as Any).currentTime).toBe(w.now);
  });

  test("document.timeline holds the frame's rAF timestamp until the next wave, and never goes back", async () => {
    const w = world();
    const timeline = w.doc.timeline as Any;
    let inFrame: Any = null;
    let ts = 0;
    w.raf((t) => {
      ts = t;
      inFrame = timeline.currentTime;
    });
    await w.frame(16);
    expect(inFrame).toBe(ts);
    w.tick(7);
    expect(timeline.currentTime).toBe(ts);
    await w.frame(16);
    expect(timeline.currentTime).toBe(w.now);
    const read = timeline.currentTime;
    w.tick(3);
    expect(timeline.currentTime).toBe(read);
    await w.frame(0);
    w.tick(5);
    const early = timeline.currentTime;
    let stamped: Any = null;
    w.raf(() => (stamped = timeline.currentTime));
    await w.frame(0);
    expect(stamped).toBeGreaterThanOrEqual(early);
  });

  test("hands the host the effect, then holds at 0 until the next frame starts the clock", async () => {
    const w = world();
    const a = w.el(7).animate([{ transform: "translateY(-100px)" }, { transform: "translateY(-120px)" }], {
      duration: 900,
      easing: "cubic-bezier(0.34, 1.56, 0.64, 1)",
    });
    const set = w.last("set");
    expect(set.node).toBe(7);
    expect(set.frames.map((f: Any) => [f.computed, f.props])).toEqual([
      [0, [["transform", "translateY(-100px)"]]],
      [1, [["transform", "translateY(-120px)"]]],
    ]);
    expect(set.timing).toMatchObject({
      delay: 0,
      endDelay: 0,
      duration: 900,
      iterations: 1,
      iterationStart: 0,
      direction: "normal",
      fill: "auto",
    });
    expect(set.easing).toBe("cubic-bezier(0.34, 1.56, 0.64, 1)");
    expect(a.pending).toBe(true);
    expect(a.playState).toBe("running");
    expect(a.startTime).toBe(null);
    expect(a.currentTime).toBe(0);
    expect(w.last("clock")).toMatchObject({ start: NaN, hold: 0, rate: 1 });

    let ready = false;
    a.ready.then((x: Any) => (ready = x === a));
    await w.frame();
    expect(ready).toBe(true);
    expect(a.pending).toBe(false);
    expect(a.startTime).toBe(w.now);
    expect(w.last("clock")).toMatchObject({ start: w.now, hold: NaN, rate: 1 });
  });

  test("finishes on time: the promise, then the event and onfinish; a non-filling one leaves the host", async () => {
    const w = world();
    const a = w.el(3).animate({ opacity: [0, 1] }, 100);
    const order: string[] = [];
    a.finished.then(() => order.push("promise"));
    a.addEventListener("finish", (e: Any) => order.push(`event:${e.currentTime}`));
    a.onfinish = () => order.push("onfinish");
    await w.frame(); // ready
    await w.frame(50);
    expect(a.playState).toBe("running");
    expect(order).toEqual([]);
    await w.frame(60);
    expect(a.playState).toBe("finished");
    expect(a.currentTime).toBe(100);
    expect(order).toEqual(["promise"]);
    await w.frame();
    expect(order).toEqual(["promise", "event:100", "onfinish"]);
    expect(w.last("drop")).toMatchObject({ id: w.last("set").id });
    expect(w.HTMLProto.getAnimations.call(w.el(3))).toEqual([]);
  });

  test("onfinish runs before a frame's rAF callbacks: an end value written from one lands with the drop", async () => {
    const w = world();
    const a = w.el(11).animate({ opacity: [1, 0.5] }, { duration: 100, fill: "both" });
    const log: string[] = [];
    let frameNo = 0;
    a.onfinish = () => {
      w.raf(() => log.push(`style@${frameNo}`));
      a.cancel();
      log.push(`drop@${frameNo}`);
    };
    for (; frameNo < 12 && !log.length; frameNo++) await w.frame(16);
    await w.frame();
    expect(log).toHaveLength(2);
    expect(log[0]!.split("@")[1]).toBe(log[1]!.split("@")[1]);
    expect(log[0]!.startsWith("drop")).toBe(true);
  });

  test("cancel mid-flight rejects `finished` with AbortError, fires cancel, and drops the effect", async () => {
    const w = world();
    const target = w.el(4);
    const a = target.animate({ transform: ["translateY(0px)", "translateY(-20px)"] }, 900);
    await w.frame();
    await w.frame(300);
    let why = "";
    a.finished.catch((e: Any) => (why = e.name));
    let cancelled = false;
    a.oncancel = () => (cancelled = true);
    expect(target.getAnimations()).toEqual([a]);
    a.cancel();
    await w.flush();
    expect(why).toBe("AbortError");
    expect(cancelled).toBe(false);
    await w.frame();
    expect(cancelled).toBe(true);
    expect(a.playState).toBe("idle");
    expect(a.currentTime).toBe(null);
    expect(w.last("drop")).toBeTruthy();
    expect(target.getAnimations()).toEqual([]);
    expect(a.finished).toBeInstanceOf(Promise);
  });

  test("pause holds the time, a seek moves it, play resumes from it", async () => {
    const w = world();
    const a = w.el(5).animate({ opacity: [0, 1] }, 1000);
    await w.frame();
    await w.frame(200);
    a.pause();
    expect(a.playState).toBe("paused");
    expect(a.pending).toBe(true);
    await w.frame(10);
    expect(a.pending).toBe(false);
    expect(a.currentTime).toBe(210);
    expect(w.last("clock")).toMatchObject({ start: NaN, hold: 210 });
    a.currentTime = 500;
    expect(w.last("clock")).toMatchObject({ hold: 500 });
    expect(a.effect.getComputedTiming().progress).toBe(0.5);
    a.play();
    await w.frame(16);
    const start = w.now - 500;
    expect(a.startTime).toBe(start);
    expect(w.last("clock")).toMatchObject({ start, hold: NaN, rate: 1 });
    await w.frame(100);
    expect(a.currentTime).toBe(600);
  });

  test("reverse plays back to 0 and finishes there; playbackRate scales the clock", async () => {
    const w = world();
    const a = w.el(6).animate({ opacity: [0, 1] }, { duration: 400, fill: "both" });
    await w.frame();
    await w.frame(100);
    a.reverse();
    await w.frame(16);
    expect(a.playbackRate).toBe(-1);
    expect(w.last("clock").rate).toBe(-1);
    let done = false;
    a.finished.then(() => (done = true));
    await w.frame(120);
    expect(done).toBe(true);
    expect(a.currentTime).toBe(0);

    const b = w.el(8).animate({ opacity: [0, 1] }, 400);
    b.playbackRate = 2;
    await w.frame();
    let fast = false;
    b.finished.then(() => (fast = true));
    await w.frame(201);
    expect(fast).toBe(true);
  });

  test("a filling animation stays until a later one covers it, then is removed", async () => {
    const w = world();
    const target = w.el(9);
    const a = target.animate({ opacity: [0, 0.5] }, { duration: 100, fill: "forwards" });
    await w.frame();
    await w.frame(110);
    expect(a.playState).toBe("finished");
    expect(target.getAnimations()).toEqual([a]);
    expect(w.calls.filter((c) => c.op === "drop")).toEqual([]);
    let removed = false;
    a.onremove = () => (removed = true);
    const b = target.animate({ opacity: [0.5, 1] }, { duration: 100, fill: "forwards" });
    await w.frame();
    await w.frame(110);
    expect(b.playState).toBe("finished");
    expect(a.replaceState).toBe("removed");
    await w.frame();
    expect(removed).toBe(true);
    expect(target.getAnimations()).toEqual([b]);
    b.persist();
    const c = target.animate({ opacity: [1, 0.2] }, { duration: 100, fill: "forwards" });
    await w.frame();
    await w.frame(110);
    expect(b.replaceState).toBe("persisted");
    expect(target.getAnimations()).toEqual([b, c]);
  });

  test("updateTiming re-sends the effect; commitStyles writes the host's values", async () => {
    const w = world();
    const target = w.el(10);
    const a = target.animate({ opacity: [0, 1] }, { duration: 100, fill: "forwards" });
    await w.frame();
    const sets = w.calls.filter((c) => c.op === "set").length;
    a.effect.updateTiming({ duration: 300, delay: 50 });
    expect(w.calls.filter((c) => c.op === "set").length).toBe(sets + 1);
    expect(w.last("set").timing).toMatchObject({
      delay: 50,
      endDelay: 0,
      duration: 300,
      iterations: 1,
      iterationStart: 0,
      direction: "normal",
      fill: "forwards",
    });
    expect(a.effect.getComputedTiming()).toMatchObject({ duration: 300, delay: 50, endTime: 350, activeDuration: 300 });
    w.values = "opacity\u001d0.5\u001ctransform\u001dtranslate(0px, -20px)";
    a.commitStyles();
    expect(w.styles).toEqual([
      ["opacity", "0.5"],
      ["transform", "translate(0px, -20px)"],
    ]);
  });

  test("an element the host does not render runs its timing and paints nothing", async () => {
    const w = world();
    const stranger: Any = Object.create(w.HTMLProto);
    const a = stranger.animate({ opacity: [0, 1] }, 100);
    expect(w.calls.filter((c) => c.op === "set")).toEqual([]);
    expect(w.warnings).toContain("waapi:target");
    let done = false;
    a.finished.then(() => (done = true));
    await w.frame();
    await w.frame(120);
    expect(done).toBe(true);
  });
});
