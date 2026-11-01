// Web Animations' object model (src/waapi.ts): `Element.animate()`, the
// `Animation` state machine, its promises and events, keyframe and timing
// processing, and what it hands the host. The host's painting half
// (crates/vn-css/src/animator/script.rs) has its own Rust tests; here it is a
// fake that records the effect and the clock it is given.

import { describe, expect, test } from "bun:test";
import { installWaapi, parseEasing } from "../src/waapi";

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any

/** A world: a controllable clock, frames and timers, a recording host, two prototypes. */
function world() {
  let now = 1000;
  const frames: ((ts: number) => void)[] = [];
  let timers: { at: number; cb: () => void; id: number }[] = [];
  let timerId = 0;
  const calls: Any[] = [];
  const warnings: string[] = [];
  const styles: [string, string][] = [];
  let values: string | null = null;
  const host = {
    set(id: number, node: number, frames: string, timing: string, easing: string) {
      calls.push({ op: "set", id, node, frames, timing, easing });
      return notes;
    },
    clock(id: number, start: number, hold: number, rate: number) {
      calls.push({ op: "clock", id, start, hold, rate });
    },
    drop(id: number) {
      calls.push({ op: "drop", id });
    },
    values(id: number) {
      calls.push({ op: "values", id });
      return values;
    },
  };
  let notes = "";
  const HTMLProto: Any = {};
  const SVGProto: Any = {};
  const ElementProto: Any = {};
  const globals: Record<string, unknown> = {};
  const doc: Record<string, unknown> = {};
  const api = installWaapi({
    host,
    protos: [HTMLProto, SVGProto],
    nodeOf: (t: Any) => t.__vnId ?? 0,
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
  const el = (id: number): Any => Object.assign(Object.create(HTMLProto), { __vnId: id, parentNode: null });
  /** One frame `ms` later: due timers, then the animation frame callbacks, then microtasks. */
  const frame = async (ms = 16) => {
    now += ms;
    const due = timers.filter((t) => t.at <= now).sort((a, b) => a.at - b.at);
    timers = timers.filter((t) => t.at > now);
    for (const t of due) t.cb();
    const batch = frames.splice(0);
    for (const cb of batch) cb(now);
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
    HTMLProto,
    SVGProto,
    ElementProto,
    get now() {
      return now;
    },
    set notes(v: string) {
      notes = v;
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
    for (const name of ["Animation", "KeyframeEffect", "AnimationEffect", "DocumentTimeline", "AnimationTimeline", "AnimationPlaybackEvent"]) {
      expect(typeof w.globals[name]).toBe("function");
    }
    expect((w.doc.timeline as Any).currentTime).toBe(w.now);
  });

  test("hands the host the effect, then holds at 0 until the next frame starts the clock", async () => {
    const w = world();
    const a = w.el(7).animate([{ transform: "translateY(-100px)" }, { transform: "translateY(-120px)" }], {
      duration: 900,
      easing: "cubic-bezier(0.34, 1.56, 0.64, 1)",
    });
    const set = w.last("set");
    expect(set.node).toBe(7);
    expect(set.frames).toBe("0\u001flinear\u001ftransform\u001dtranslateY(-100px)\u001e1\u001flinear\u001ftransform\u001dtranslateY(-120px)");
    expect(set.timing).toBe("0,0,900,1,0,0,0");
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
    expect(order).toEqual(["promise", "event:100", "onfinish"]);
    expect(w.last("drop")).toMatchObject({ id: w.last("set").id });
    expect(w.HTMLProto.getAnimations.call(w.el(3))).toEqual([]);
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
    expect(cancelled).toBe(true);
    expect(a.playState).toBe("idle");
    expect(a.currentTime).toBe(null);
    expect(w.last("drop")).toBeTruthy();
    expect(target.getAnimations()).toEqual([]);
    // A fresh finished promise, as the DOM has it.
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
    expect(removed).toBe(true);
    expect(target.getAnimations()).toEqual([b]);
    // persist() keeps one.
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
    expect(w.last("set").timing).toBe("50,0,300,1,0,0,1");
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

  test("the host's notes become advisories", () => {
    const w = world();
    w.notes = "Wfilter\nW--x";
    w.el(1).animate({ filter: ["blur(0px)", "blur(4px)"], "--x": [0, 1] } as Any, 100);
    expect(w.warnings).toContain("waapi:property:filter");
    expect(w.warnings).toContain("waapi:property:--x");
  });
});

describe("keyframes and timing", () => {
  test("the list form computes missing offsets and refuses ones that go backwards", () => {
    const w = world();
    const E = w.globals.KeyframeEffect as Any;
    const e = new E(null, [{ opacity: 0 }, { opacity: 0.2 }, { opacity: 0.4, offset: 0.8 }, { opacity: 1 }]);
    expect(e.getKeyframes().map((k: Any) => k.computedOffset)).toEqual([0, 0.4, 0.8, 1]);
    expect(e.getKeyframes()[2].offset).toBe(0.8);
    expect(() => new E(null, [{ opacity: 0, offset: 0.5 }, { opacity: 1, offset: 0.2 }])).toThrow(TypeError);
    expect(() => new E(null, [{ opacity: 0, offset: 1.5 }])).toThrow(TypeError);
    // A single keyframe is the end one.
    expect(new E(null, [{ opacity: 1 }]).getKeyframes()[0].computedOffset).toBe(1);
  });

  test("the property-indexed form spaces each property's values and merges them", () => {
    const w = world();
    const E = w.globals.KeyframeEffect as Any;
    const e = new E(null, { opacity: [0, 1], transform: ["none", "scale(2)", "none"], easing: "ease-in" });
    const k = e.getKeyframes();
    expect(k.map((f: Any) => f.computedOffset)).toEqual([0, 0.5, 1]);
    expect(k[0]).toMatchObject({ opacity: "0", transform: "none", easing: "ease-in" });
    expect(k[1]).toMatchObject({ transform: "scale(2)" });
    expect(k[1].opacity).toBeUndefined();
    // `{ opacity: [1] }` — motion's partial-keyframes probe — is legal.
    expect(new E(null, { opacity: [1] }).getKeyframes()[0].computedOffset).toBe(1);
  });

  test("invalid timing and easing throw TypeError, linear() among them", () => {
    const w = world();
    const node = w.el(1);
    expect(() => node.animate({ opacity: 0 }, { easing: "linear(0, 1)" })).toThrow(TypeError);
    expect(() => node.animate({ opacity: 0 }, { duration: -1 })).toThrow(TypeError);
    expect(() => node.animate({ opacity: 0 }, { fill: "sideways" })).toThrow(TypeError);
    expect(() => node.animate({ opacity: 0 }, { iterations: NaN })).toThrow(TypeError);
    expect(() => node.animate([{ opacity: 0, easing: "bounce" }])).toThrow(TypeError);
    expect(() => node.animate({ opacity: 0 }, { duration: "auto", direction: "alternate", iterations: Infinity })).not.toThrow();
  });

  test("hyphenated names are ignored with an advisory, camelCase becomes CSS", () => {
    const w = world();
    w.el(1).animate({ "background-color": ["red", "blue"], backgroundColor: ["red", "blue"] } as Any, 100);
    expect(w.last("set").frames).toBe("0\u001flinear\u001fbackground-color\u001dred\u001e1\u001flinear\u001fbackground-color\u001dblue");
    expect(w.warnings).toContain("waapi:hyphen:background-color");
  });

  test("the timing model: delay, fill, iterations, direction", async () => {
    const w = world();
    const a = w.el(1).animate({ opacity: [0, 1] }, { duration: 100, delay: 50, iterations: 3, direction: "alternate", fill: "both" });
    a.pause();
    const t = (ms: number) => {
      a.currentTime = ms;
      return a.effect.getComputedTiming();
    };
    expect(t(0)).toMatchObject({ progress: 0, currentIteration: 0 });
    expect(t(75)).toMatchObject({ progress: 0.25, currentIteration: 0 });
    expect(t(175)).toMatchObject({ progress: 0.75, currentIteration: 1 });
    expect(t(275)).toMatchObject({ progress: 0.25, currentIteration: 2 });
    expect(t(1000)).toMatchObject({ progress: 1, currentIteration: 2 });
    expect(a.effect.getComputedTiming()).toMatchObject({ endTime: 350, activeDuration: 300 });
  });

  test("easings evaluate like CSS", () => {
    const close = (a: number, b: number) => expect(Math.abs(a - b)).toBeLessThan(1e-3);
    close(parseEasing("ease")!(0.5), 0.8024);
    close(parseEasing("ease-in-out")!(0.5), 0.5);
    close(parseEasing("cubic-bezier(0.34, 1.56, 0.64, 1)")!(0.5), 1.0874);
    close(parseEasing("cubic-bezier(0.34, 1.56, 0.64, 1)")!(0.25), 0.8163);
    expect(parseEasing("steps(4, end)")!(0.6)).toBe(0.5);
    expect(parseEasing("steps(4, jump-start)")!(0.6)).toBe(0.75);
    expect(parseEasing("step-end")!(0.99)).toBe(0);
    expect(parseEasing("cubic-bezier(2, 0, 0, 1)")).toBe(null);
    expect(parseEasing("linear(0, 1)")).toBe(null);
  });
});
