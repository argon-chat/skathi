// <Transition> / <TransitionGroup>: they must EXIST as exports (their absence is a hard prod-build failure for
// anything importing them from "vue", which @vueuse/core does at module scope), and the class pipeline must run
// the same phases in the same order, riding sk-css's real transition engine.

import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

type Rt = ReturnType<typeof createRuntime>;
interface PhaseHooks {
  onBeforeEnter(el: MirrorElement): void;
  onBeforeAppear(el: MirrorElement): void;
  onEnter(el: MirrorElement, done: () => void): void;
  onLeave(el: MirrorElement, done: () => void): void;
}
const phaseHooks = (rt: Rt, props: Parameters<Rt["resolveTransitionProps"]>[0]) =>
  rt.resolveTransitionProps(props) as unknown as PhaseHooks;

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("the class pipeline", () => {
  function el(rt: Rt) {
    const root = rt.nodeOps.createElement("root");
    const node = rt.nodeOps.createElement("div");
    rt.nodeOps.insert(node, root, null);
    rt.patchProp(node, "class", null, "card");
    return node;
  }
  const lastClass = (rt: Rt) => rt.host.classes.at(-1)?.value;

  test("enter runs from-classes, then to-classes on the next frame", async () => {
    const rt = createRuntime();
    const node = el(rt);
    const hooks = phaseHooks(rt, { name: "fade" });

    hooks.onBeforeEnter(node);
    expect(lastClass(rt)).toBe("card fade-enter-from fade-enter-active");

    let done = false;
    hooks.onEnter(node, () => (done = true));
    expect(lastClass(rt)).toContain("fade-enter-from");

    await tick();
    await tick();
    await tick();
    expect(rt.host.classes.map((c) => c.value)).toContain("card fade-enter-active fade-enter-to");
    expect(done).toBe(true);
    expect(lastClass(rt)).toBe("card");
  });

  test("leave runs the leave phases and clears them when done", async () => {
    const rt = createRuntime();
    const node = el(rt);
    const hooks = phaseHooks(rt, { name: "fade" });
    let done = false;
    hooks.onLeave(node, () => (done = true));
    expect(lastClass(rt)).toBe("card fade-leave-from fade-leave-active");
    await tick();
    await tick();
    await tick();
    expect(done).toBe(true);
    expect(lastClass(rt)).toBe("card");
  });

  test("`appear` uses its own class names when given", () => {
    const rt = createRuntime();
    const node = el(rt);
    const hooks = phaseHooks(rt, { name: "fade", appearFromClass: "boot" });
    hooks.onBeforeAppear(node);
    expect(lastClass(rt)).toBe("card boot fade-enter-active");
  });

  test("`css: false` leaves the class pipeline out entirely", () => {
    const rt = createRuntime();
    const onEnter = () => {};
    const props = rt.resolveTransitionProps({ name: "fade", css: false, onEnter, mode: "out-in" });
    expect(props.onEnter).toBe(onEnter);
    expect(props.mode).toBe("out-in");
  });

  test("the end of a phase is timed from the element's computed transition-duration", async () => {
    const rt = createRuntime();
    const node = el(rt);
    rt.host.computedById.set(
      node.__skId,
      "transition-duration=0.05s\ntransition-delay=0s\ntransition-property=all\n" +
        "animation-name=none\nanimation-duration=0s\nanimation-delay=0s\n",
    );
    const hooks = phaseHooks(rt, { name: "fade" });
    let done = false;
    hooks.onEnter(node, () => (done = true));
    await tick();
    await tick();
    await tick();
    expect(done).toBe(false);
    expect(lastClass(rt)).toContain("fade-enter-to");
    await new Promise((r) => setTimeout(r, 80));
    expect(done).toBe(true);
    expect(rt.advisories()).toEqual([]);
  });

  test("a phase run by a CSS animation ends on its animationend, not at once", async () => {
    const rt = createRuntime();
    const node = el(rt);
    rt.host.computedById.set(
      node.__skId,
      "transition-duration=0s\nanimation-name=pop\nanimation-duration=5s\nanimation-delay=0s\n",
    );
    const hooks = phaseHooks(rt, { name: "pop" });
    let done = false;
    hooks.onEnter(node, () => (done = true));
    await tick();
    await tick();
    await tick();
    node.dispatchEvent(new CustomEvent("transitionend", { bubbles: true }));
    expect(done).toBe(false);
    node.dispatchEvent(new CustomEvent("animationend", { bubbles: true }));
    expect(done).toBe(true);
  });

  test("a real transitionend ends the phase before the timer would", async () => {
    const rt = createRuntime();
    const node = el(rt);
    rt.host.computedById.set(node.__skId, "transition-duration=5s\ntransition-delay=0s\n");
    const hooks = phaseHooks(rt, { name: "fade" });
    let done = false;
    hooks.onEnter(node, () => (done = true));
    await tick();
    await tick();
    await tick();
    expect(done).toBe(false);
    node.dispatchEvent(new CustomEvent("transitionend", { bubbles: true }));
    expect(done).toBe(true);
  });

  test("an explicit `duration` wins over the computed one", async () => {
    const rt = createRuntime();
    const node = el(rt);
    rt.host.computedById.set(node.__skId, "transition-duration=10s\n");
    const hooks = phaseHooks(rt, { name: "fade", duration: 10 });
    let done = false;
    hooks.onEnter(node, () => (done = true));
    await new Promise((r) => setTimeout(r, 60));
    expect(done).toBe(true);
  });

  test("a hook with an explicit `done` argument suppresses the automatic timing", async () => {
    const rt = createRuntime();
    const node = el(rt);
    rt.host.computedById.set(node.__skId, "transition-duration=0.05s\n");
    const hooks = phaseHooks(rt, { name: "fade", onEnter: (_el: unknown, _done: unknown) => {} });
    let done = false;
    hooks.onEnter(node, () => (done = true));
    await new Promise((r) => setTimeout(r, 90));
    expect(done).toBe(false);
  });
});
