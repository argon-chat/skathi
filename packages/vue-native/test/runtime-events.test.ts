// The renderer's event-prop plumbing: which `on*` props reach the host, which
// names they normalise to, and — the compat-first part — that a prop the host
// cannot deliver says so instead of vanishing.

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

describe("patchProp: on* props", () => {
  test("a known event registers its listener flag under the host's code", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(7), "onClick", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: 7, code: 1, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("an event the host does not deliver warns instead of vanishing", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(), "onContextmenu", null, () => {});
    const advisories = rt.advisories();
    expect(advisories.length).toBe(1);
    // Same shape as the addEventListener advisory next to it.
    expect(advisories[0]).toContain("@contextmenu");
    expect(advisories[0]).toContain("not delivered by the native host yet");
    expect(advisories[0]).toContain("will never fire");
    // …and no listener flag was set, because there is no code to set it under.
    expect(rt.host.listeners).toEqual([]);
  });

  test("the advisory is one per event name per session, not one per patch", () => {
    const rt = createRuntime();
    for (let i = 0; i < 5; i++) rt.patchProp(rt.el(i), "onContextmenu", null, () => {});
    rt.patchProp(rt.el(), "onDblclick", null, () => {});
    expect(rt.advisories().length).toBe(2);
    // A *different* world starts over — the dedup set is per module instance.
    const other = createRuntime();
    other.patchProp(other.el(), "onContextmenu", null, () => {});
    expect(other.advisories().length).toBe(1);
  });

  test("addEventListener and the prop path share one dedup key", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(3), "onContextmenu", null, () => {});
    const node = rt.nodeOps.createElement("div");
    node.addEventListener("contextmenu", () => {});
    expect(rt.advisories().length).toBe(1);
  });

  test("component-emit props that leak onto an element stay silent", () => {
    // `<input v-model>` compiles to an `onUpdate:modelValue` prop on the
    // ELEMENT. A browser binds it to an event that never fires, silently; the
    // advisory is for real DOM event names, not for this.
    const rt = createRuntime();
    rt.patchProp(rt.el(), "onUpdate:modelValue", null, () => {});
    expect(rt.advisories()).toEqual([]);
  });

  test("a window-only event on an ELEMENT is silent — a browser never fires it there either", () => {
    // `el.addEventListener("resize", …)` is dead code in Chrome too: `resize`
    // is a `window` event. Warning here would report a divergence that does not
    // exist, and a component that hedges by attaching to both targets would
    // cost a log line on every app. On `window` the listener is real — it falls
    // through to redjs's own EventTarget, where the host's trusted `resize` is
    // dispatched.
    const rt = createRuntime();
    const node = rt.nodeOps.createElement("div");
    for (const name of ["resize", "load", "beforeunload", "hashchange", "storage"]) {
      node.addEventListener(name, () => {});
    }
    expect(rt.advisories()).toEqual([]);
    // An element event the host really cannot deliver still speaks.
    node.addEventListener("contextmenu", () => {});
    expect(rt.advisories().length).toBe(1);
  });

  test("modifiers are stripped before the name is looked up", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(2), "onClickCaptureOnce", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: 2, code: 1, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });
});

describe("<lottie> event names", () => {
  test("the host codes are in lockstep with crates/vn-tree/src/tree.rs", () => {
    const rt = createRuntime();
    expect(rt.EV_CODES.complete).toBe(11);
    expect(rt.EV_CODES.loopComplete).toBe(12);
    expect(rt.EV_CODES.enterFrame).toBe(13);
  });

  test("the `on:` form the compiler emits for a custom element keeps its case", () => {
    // `@loopComplete` on <lottie> compiles to the prop `on:loopComplete` —
    // compiler-dom's case-preserving spelling for a custom element. That is the
    // shape this element actually receives, so it is the one that must work.
    const rt = createRuntime();
    rt.patchProp(rt.el(4), "on:loopComplete", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.loopComplete)).toEqual([{ id: 4, code: 12, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("the hyphenated spelling folds onto the same name", () => {
    // If the tag is ever classified as a plain element, `@loopComplete` becomes
    // `onLoopComplete` and runtime-dom hyphenates it to "loop-complete" — the
    // well-known reason a camelCase custom-element event is missed in the
    // browser. We fold it back rather than dropping the listener.
    const rt = createRuntime();
    expect(rt.hyphenate("LoopComplete")).toBe("loop-complete");
    expect(rt.canonicalEvent("loop-complete")).toBe("loopComplete");
    rt.patchProp(rt.el(4), "onLoopComplete", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.loopComplete)).toEqual([{ id: 4, code: 12, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("an unknown `on:` event still gets its advisory", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(), "on:someCustomThing", null, () => {});
    expect(rt.advisories().length).toBe(1);
    expect(rt.advisories()[0]).toContain("@someCustomThing");
  });

  test("addEventListener uses lottie-web's own camelCase spelling", () => {
    const rt = createRuntime();
    const node = rt.nodeOps.createElement("lottie");
    node.addEventListener("loopComplete", () => {});
    expect(rt.flagsFor(rt.EV_CODES.loopComplete).length).toBe(1);
    expect(rt.advisories()).toEqual([]);
  });

  test("both spellings land in ONE bucket, so removing works across them", () => {
    const rt = createRuntime();
    const node = rt.nodeOps.createElement("lottie");
    const fn = () => {};
    node.addEventListener("enter-frame", fn);
    expect(rt.flagsFor(rt.EV_CODES.enterFrame).at(-1).on).toBe(true);
    node.removeEventListener("enterFrame", fn);
    expect(rt.flagsFor(rt.EV_CODES.enterFrame).at(-1).on).toBe(false);
  });
});

// crates/vn-video/CONTRACT.md §4 and §6. The Rust side has dispatched these
// eighteen since <video> landed; until this file knew their names they arrived
// nowhere and `@timeupdate` warned.
describe("<video> media events and the HTMLMediaElement surface", () => {
  const STATE = [
    "currentTime=3.5",
    "duration=12",
    "paused=false",
    "ended=false",
    "seeking=false",
    "readyState=4",
    "networkState=1",
    "volume=0.5",
    "muted=true",
    "playbackRate=2",
    "loop=true",
    "videoWidth=1920",
    "videoHeight=1080",
    "buffered=0:6.25",
    "seekable=0:12",
    "error=0",
    "errorMessage=",
  ].join("\n");

  const video = (rt) => {
    const node = rt.nodeOps.createElement("video");
    rt.host.mediaById.set(node.id, STATE);
    return node;
  };

  test("the media codes are in lockstep with crates/vn-tree/src/tree.rs", () => {
    const rt = createRuntime();
    expect(rt.EV_CODES.loadstart).toBe(17);
    expect(rt.EV_CODES.timeupdate).toBe(27);
    expect(rt.EV_CODES.ended).toBe(32);
    expect(rt.EV_CODES.emptied).toBe(34);
    // …and the animator's four sit AFTER the media block, not on top of it.
    expect(rt.EV_CODES.transitionend).toBe(35);
    expect(rt.EV_CODES.animationcancel).toBe(38);
  });

  test("@timeupdate registers a real host listener instead of advising", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(9), "onTimeupdate", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.timeupdate)).toEqual([{ id: 9, code: 27, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("a dispatched timeupdate carries the payload and does not bubble", () => {
    const rt = createRuntime();
    const node = video(rt);
    const seen = [];
    node.addEventListener("timeupdate", (e) => seen.push(e));
    rt.host.dispatcher(node.id, rt.EV_CODES.timeupdate, 3.5, 12, 4, 2, "");
    expect(seen.length).toBe(1);
    expect(seen[0].currentTime).toBe(3.5);
    expect(seen[0].detail.duration).toBe(12);
    expect(seen[0].bubbles).toBe(false);
  });

  test("the properties read live off the host, per call", () => {
    const rt = createRuntime();
    const node = video(rt);
    expect(node.currentTime).toBe(3.5);
    expect(node.duration).toBe(12);
    expect(node.paused).toBe(false);
    expect(node.muted).toBe(true);
    expect(node.playbackRate).toBe(2);
    expect(node.videoWidth).toBe(1920);
    expect(node.buffered.length).toBe(1);
    expect(node.buffered.end(0)).toBe(6.25);
    expect(node.error).toBe(null);
    // Live: the host moved, and so does the property. No cache to invalidate.
    rt.host.mediaById.set(node.id, STATE.replace("currentTime=3.5", "currentTime=9"));
    expect(node.currentTime).toBe(9);
  });

  test("the transport verbs reach __vn_media_call", () => {
    const rt = createRuntime();
    const node = video(rt);
    node.play();
    node.currentTime = 8;
    node.muted = false;
    node.pause();
    expect(rt.host.mediaCalls).toEqual([
      { id: node.id, verb: 0, arg: 0 },
      { id: node.id, verb: 2, arg: 8 },
      { id: node.id, verb: 4, arg: 0 },
      { id: node.id, verb: 1, arg: 0 },
    ]);
  });

  test("an errored element reports a MediaError with the advisory text", () => {
    const rt = createRuntime();
    const node = video(rt);
    rt.host.mediaById.set(
      node.id,
      "error=4\nerrorMessage=this machine has no HEVC decoder; re-encode to H.264",
    );
    expect(node.error.code).toBe(4);
    expect(node.error.message).toContain("H.264");
  });

  test("canPlayType is a live probe, and a non-media element has no surface", () => {
    const rt = createRuntime();
    const node = video(rt);
    rt.host.canPlayByType.set('video/mp4; codecs="avc1.640028"', "probably");
    expect(node.canPlayType('video/mp4; codecs="avc1.640028"')).toBe("probably");
    expect(node.canPlayType("application/x-nonsense")).toBe("");
    // A <div> is not an HTMLMediaElement in a browser either.
    expect(rt.nodeOps.createElement("div").currentTime).toBe(undefined);
  });
});
