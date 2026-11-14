// `el.animate()` wired into the runtime (src/runtime.ts → src/waapi.ts): which
// prototypes carry it, what a mirror element hands the host, and the inert
// path on a host without `__sk_anim_*`. The state machine itself is
// test/waapi.test.ts; the painting half is crates/sk-css/src/animator/script.rs.

import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

interface TestAnimation {
  ready: Promise<unknown>;
  finished: Promise<unknown>;
  startTime: unknown;
  playState: string;
  commitStyles(): void;
}
type AnimatableElement = MirrorElement & {
  animate(keyframes: unknown, options: unknown): TestAnimation;
  getAnimations(): TestAnimation[];
};
interface AnimationGlobal {
  [name: string]: unknown;
  document: { timeline: { currentTime: unknown }; getAnimations(): unknown[] };
  requestAnimationFrame(cb: (ts: number) => void): void;
  CSS: { supports(...args: string[]): boolean };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe("el.animate() in the runtime", () => {
  test("a mirror element animates through the host, with one placement advisory", async () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("div") as AnimatableElement;
    expect(typeof el.animate).toBe("function");
    expect(typeof el.getAnimations).toBe("function");
    const a = el.animate([{ transform: "translateY(0px)" }, { transform: "translateY(-24px)" }], {
      duration: 30,
      easing: "ease-out",
    });
    const set = rt.host.anim.calls.find((c) => c[0] === "set")!;
    expect(set[2]).toBe(el.__skId);
    expect((set[3] as { computed: number; props: unknown }[]).map((f) => [f.computed, f.props])).toEqual([
      [0, [["transform", "translateY(0px)"]]],
      [1, [["transform", "translateY(-24px)"]]],
    ]);
    expect(set[4]).toMatchObject({
      delay: 0,
      endDelay: 0,
      duration: 30,
      iterations: 1,
      iterationStart: 0,
      direction: "normal",
      fill: "auto",
    });
    expect(set[5]).toBe("ease-out");
    expect(rt.host.anim.calls.find((c) => c[0] === "clock")).toEqual(["clock", set[1], NaN, 0, 1]);
    expect(rt.advisories().filter((l) => l.includes("Element.animate() runs natively"))).toHaveLength(1);
    expect(el.getAnimations()).toEqual([a]);
    await a.ready;
    expect(typeof a.startTime).toBe("number");
    await a.finished;
    expect(a.playState).toBe("finished");
    expect(rt.host.anim.calls.some((c) => c[0] === "drop" && c[1] === set[1])).toBe(true);
    expect(el.getAnimations()).toEqual([]);
  });

  test("commitStyles writes the host's values into the inline style", async () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("div") as AnimatableElement;
    const a = el.animate({ opacity: [0, 0.5] }, { duration: 10, fill: "forwards" });
    await a.finished;
    rt.host.anim.values = "opacity\u001d0.5";
    a.commitStyles();
    expect(el.style.opacity).toBe("0.5");
  });

  test("the globals and document.timeline", () => {
    const rt = createRuntime();
    const g = rt.global as unknown as AnimationGlobal;
    for (const name of [
      "Animation",
      "KeyframeEffect",
      "AnimationEffect",
      "DocumentTimeline",
      "AnimationTimeline",
      "AnimationPlaybackEvent",
    ]) {
      expect(typeof g[name]).toBe("function");
    }
    expect(typeof g.document.timeline.currentTime).toBe("number");
    expect(g.document.getAnimations()).toEqual([]);
  });

  test("CSS.supports says no to linear() easing, which el.animate() refuses", () => {
    // Argon's Digit.vue asks exactly this, and builds a linear() spring when
    // the answer is yes: el.animate() would then throw on every tick.
    const rt = createRuntime();
    const CSS = (rt.global as unknown as AnimationGlobal).CSS;
    expect(CSS.supports("animation-timing-function", "linear(0, 1)")).toBe(false);
    expect(CSS.supports("transition-timing-function: LINEAR(0, 0.5 50%, 1)")).toBe(false);
    expect(CSS.supports("transition", "opacity 1s linear(0, 1)")).toBe(false);
    expect(CSS.supports("animation-timing-function", "cubic-bezier(0.34, 1.56, 0.64, 1)")).toBe(true);
    expect(CSS.supports("transition-timing-function", "linear")).toBe(true);
    expect(CSS.supports("background", "linear-gradient(red, blue)")).toBe(true);
    const el = rt.nodeOps.createElement("div") as AnimatableElement;
    expect(() => el.animate({ opacity: [0, 1] }, { duration: 10, easing: "linear(0, 1)" })).toThrow(TypeError);
  });

  test("a rAF callback stamps document.timeline with its timestamp", async () => {
    const TS = 1e9 + 0.5;
    const rt = createRuntime({
      globals: { requestAnimationFrame: (cb: (ts: number) => void) => setTimeout(() => cb(TS), 0) },
    });
    const g = rt.global as unknown as AnimationGlobal;
    const seen = await new Promise((resolve) =>
      g.requestAnimationFrame((ts: number) => resolve([ts, g.document.timeline.currentTime])),
    );
    expect(seen).toEqual([TS, TS]);
    // Still held after the callback, in its wave.
    expect(g.document.timeline.currentTime).toBe(TS);
  });

  test("an older host without __sk_anim_* installs nothing", async () => {
    const rt = createRuntime({ anim: false });
    const el = rt.nodeOps.createElement("div") as AnimatableElement;
    expect((el as Partial<AnimatableElement>).animate).toBeUndefined();
    expect((rt.global as Record<string, unknown>).KeyframeEffect).toBeUndefined();
    await sleep(0);
  });
});
