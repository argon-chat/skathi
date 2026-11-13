// The renderer's event-prop plumbing: which `on*` props reach the host, which names they normalise to, and that a prop the host cannot deliver says so instead of vanishing.

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
    rt.patchProp(rt.el(), "onMouseover", null, () => {});
    const advisories = rt.advisories();
    expect(advisories.length).toBe(1);
    expect(advisories[0]).toContain("@mouseover");
    expect(advisories[0]).toContain("not delivered by the native host yet");
    expect(advisories[0]).toContain("will never fire");
    expect(rt.host.listeners).toEqual([]);
  });

  test("the advisory is one per event name per session, not one per patch", () => {
    const rt = createRuntime();
    for (let i = 0; i < 5; i++) rt.patchProp(rt.el(i), "onMouseover", null, () => {});
    rt.patchProp(rt.el(), "onTouchstart", null, () => {});
    expect(rt.advisories().length).toBe(2);
    const other = createRuntime();
    other.patchProp(other.el(), "onMouseover", null, () => {});
    expect(other.advisories().length).toBe(1);
  });

  test("addEventListener and the prop path share one dedup key", () => {
    const rt = createRuntime();
    rt.patchProp(rt.el(3), "onMouseover", null, () => {});
    const node = rt.nodeOps.createElement("div");
    node.addEventListener("mouseover", () => {});
    expect(rt.advisories().length).toBe(1);
  });

  test("component-emit props that leak onto an element stay silent", () => {
    // `<input v-model>` compiles to an `onUpdate:modelValue` prop on the ELEMENT: not a DOM event, so no advisory.
    const rt = createRuntime();
    rt.patchProp(rt.el(), "onUpdate:modelValue", null, () => {});
    expect(rt.advisories()).toEqual([]);
  });

  test("a window-only event on an ELEMENT is silent — a browser never fires it there either", () => {
    const rt = createRuntime();
    const node = rt.nodeOps.createElement("div");
    for (const name of ["resize", "load", "beforeunload", "hashchange", "storage"]) {
      node.addEventListener(name, () => {});
    }
    expect(rt.advisories()).toEqual([]);
    node.addEventListener("mouseover", () => {});
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
    const rt = createRuntime();
    rt.patchProp(rt.el(4), "on:loopComplete", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.loopComplete)).toEqual([{ id: 4, code: 12, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("`once`, `online` and `only` are attributes, not listeners for ce / line / ly", () => {
    const rt = createRuntime();
    for (const key of ["once", "online", "only"]) rt.patchProp(rt.el(5), key, null, "");
    expect(rt.host.listeners).toEqual([]);
    expect(rt.advisories()).toEqual([]);
    expect(rt.host.props.filter((p: { id: number }) => p.id === 5).map((p: { key: string }) => p.key)).toEqual([
      "once",
      "online",
      "only",
    ]);
  });

  test("the hyphenated spelling folds onto the same name", () => {
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
    rt.host.mediaById.set(node.__vnId, STATE);
    return node;
  };

  test("the media codes are in lockstep with crates/vn-tree/src/tree.rs", () => {
    const rt = createRuntime();
    expect(rt.EV_CODES.loadstart).toBe(17);
    expect(rt.EV_CODES.timeupdate).toBe(27);
    expect(rt.EV_CODES.ended).toBe(32);
    expect(rt.EV_CODES.emptied).toBe(34);
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
    rt.host.dispatcher(node.__vnId, rt.EV_CODES.timeupdate, 3.5, 12, 4, 2, "");
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
    rt.host.mediaById.set(node.__vnId, STATE.replace("currentTime=3.5", "currentTime=9"));
    expect(node.currentTime).toBe(9);
  });

  test("a scalar property is one host call for its own value, not the whole state", () => {
    const rt = createRuntime();
    const node = video(rt);
    rt.host.mediaStateReads = 0;
    expect([node.readyState, node.paused, node.loop, node.duration]).toEqual([4, false, true, 12]);
    expect(rt.host.mediaStateReads).toBe(0);
    expect(rt.host.mediaPropReads).toEqual([
      [node.__vnId, 5],
      [node.__vnId, 2],
      [node.__vnId, 10],
      [node.__vnId, 1],
    ]);
    expect(node.buffered.length).toBe(1);
    expect(rt.host.mediaStateReads).toBe(1);
    const bare = rt.nodeOps.createElement("video");
    expect([bare.paused, bare.readyState, bare.duration]).toEqual([true, 0, NaN]);
  });

  test("the transport verbs reach __vn_media_call", () => {
    const rt = createRuntime();
    const node = video(rt);
    node.play();
    node.currentTime = 8;
    node.muted = false;
    node.pause();
    expect(rt.host.mediaCalls).toEqual([
      { id: node.__vnId, verb: 0, arg: 0 },
      { id: node.__vnId, verb: 2, arg: 8 },
      { id: node.__vnId, verb: 4, arg: 0 },
      { id: node.__vnId, verb: 1, arg: 0 },
    ]);
  });

  test("an errored element reports a MediaError with the advisory text", () => {
    const rt = createRuntime();
    const node = video(rt);
    rt.host.mediaById.set(
      node.__vnId,
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
    expect(rt.nodeOps.createElement("div").currentTime).toBe(undefined);
  });
});

describe("the listener flag is only written when it changes", () => {
  test("re-patching a handler does not re-announce the bit", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("div");
    rt.patchProp(el, "onClick", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: el.__vnId, code: 1, on: true }]);

    for (let i = 0; i < 10; i++) rt.patchProp(el, "onClick", () => {}, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click).length).toBe(1);
  });

  test("dropping the last handler clears the bit exactly once", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("div");
    rt.patchProp(el, "onClick", null, () => {});
    rt.patchProp(el, "onClick", () => {}, null);
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([
      { id: el.__vnId, code: 1, on: true },
      { id: el.__vnId, code: 1, on: false },
    ]);
    rt.patchProp(el, "onClick", null, null);
    expect(rt.flagsFor(rt.EV_CODES.click).length).toBe(2);
  });

  test("a second listener for the same code rides the first one's bit", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("div");
    rt.patchProp(el, "onMousedown", null, () => {});
    rt.patchProp(el, "onPointerdown", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.mousedown)).toEqual([{ id: el.__vnId, code: 2, on: true }]);
  });

  test("each node keeps its own record", () => {
    const rt = createRuntime();
    const a = rt.nodeOps.createElement("div");
    const b = rt.nodeOps.createElement("div");
    rt.patchProp(a, "onClick", null, () => {});
    rt.patchProp(b, "onClick", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([
      { id: a.__vnId, code: 1, on: true },
      { id: b.__vnId, code: 1, on: true },
    ]);
  });
});

describe("v-model", () => {
  const bind = (rt, el, modifiers, onUpdate) =>
    rt.vModelText.created(el, { modifiers }, { props: { "onUpdate:modelValue": onUpdate } });

  test("updates the model on input", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input");
    let model = null;
    bind(rt, el, {}, (v) => (model = v));
    rt.host.dispatcher(el.__vnId, rt.EV_CODES.input, 0, 0, 0, 0, "typed");
    expect(model).toBe("typed");
    expect(rt.advisories()).toEqual([]);
  });

  test(".lazy still updates the model, and says how it differs", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input");
    let model = null;
    bind(rt, el, { lazy: true }, (v) => (model = v));
    rt.host.dispatcher(el.__vnId, rt.EV_CODES.input, 0, 0, 0, 0, "typed");
    expect(model).toBe("typed");
    const advisories = rt.advisories();
    expect(advisories.length).toBe(1);
    expect(advisories[0]).toContain("v-model.lazy updates on every keystroke");
    expect(advisories[0]).toContain("does not source a `change` event");
  });

  test(".number and .trim still apply", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input");
    let model = null;
    bind(rt, el, { number: true, trim: true }, (v) => (model = v));
    rt.host.dispatcher(el.__vnId, rt.EV_CODES.input, 0, 0, 0, 0, "  42  ");
    expect(model).toBe(42);
  });
});
