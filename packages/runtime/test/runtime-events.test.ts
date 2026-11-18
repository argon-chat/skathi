// The renderer's event-prop plumbing: which `on*` props reach the host, which names they normalise to, and that a prop the host cannot deliver says so instead of vanishing.

import { describe, expect, test } from "bun:test";
import { createRuntime, type MirrorElement } from "./runtime-harness";

type Rt = ReturnType<typeof createRuntime>;
type MediaEv = Event & { currentTime: number; detail: { duration: number } };

const stub = (rt: Rt, id?: number) => rt.el(id) as unknown as MirrorElement;

describe("patchProp: on* props", () => {
  test("a known event registers its listener flag under the host's code", () => {
    const rt = createRuntime();
    rt.patchProp(stub(rt, 7), "onClick", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: 7, code: 1, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("an event the host does not deliver warns instead of vanishing", () => {
    const rt = createRuntime();
    rt.patchProp(stub(rt), "onTouchend", null, () => {});
    expect(rt.advisories().length).toBe(1);
    expect(rt.host.listeners).toEqual([]);
  });

  test("the advisory is one per event name per session, not one per patch", () => {
    const rt = createRuntime();
    for (let i = 0; i < 5; i++) rt.patchProp(stub(rt, i), "onTouchend", null, () => {});
    rt.patchProp(stub(rt), "onTouchstart", null, () => {});
    expect(rt.advisories().length).toBe(2);
    const other = createRuntime();
    other.patchProp(stub(other), "onTouchend", null, () => {});
    expect(other.advisories().length).toBe(1);
  });

  test("modifiers are stripped before the name is looked up", () => {
    const rt = createRuntime();
    rt.patchProp(stub(rt, 2), "onClickCaptureOnce", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: 2, code: 1, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });
});

describe("<lottie> event names", () => {
  test("the `on:` form the compiler emits for a custom element keeps its case", () => {
    const rt = createRuntime();
    rt.patchProp(stub(rt, 4), "on:loopComplete", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.loopComplete)).toEqual([{ id: 4, code: 12, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("`once`, `online` and `only` are attributes, not listeners for ce / line / ly", () => {
    const rt = createRuntime();
    for (const key of ["once", "online", "only"]) rt.patchProp(stub(rt, 5), key, null, "");
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
    rt.patchProp(stub(rt, 4), "onLoopComplete", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.loopComplete)).toEqual([{ id: 4, code: 12, on: true }]);
    expect(rt.advisories()).toEqual([]);
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
    expect(rt.flagsFor(rt.EV_CODES.enterFrame).at(-1)?.on).toBe(true);
    node.removeEventListener("enterFrame", fn);
    expect(rt.flagsFor(rt.EV_CODES.enterFrame).at(-1)?.on).toBe(false);
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

  const video = (rt: Rt) => {
    const node = rt.nodeOps.createElement("video");
    rt.host.mediaById.set(node.__skId, STATE);
    return node;
  };

  test("@timeupdate registers a real host listener instead of advising", () => {
    const rt = createRuntime();
    rt.patchProp(stub(rt, 9), "onTimeupdate", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.timeupdate)).toEqual([{ id: 9, code: 27, on: true }]);
    expect(rt.advisories()).toEqual([]);
  });

  test("a dispatched timeupdate carries the payload and does not bubble", () => {
    const rt = createRuntime();
    const node = video(rt);
    const seen: MediaEv[] = [];
    node.addEventListener("timeupdate", (e) => seen.push(e as MediaEv));
    rt.host.dispatcher(node.__skId, rt.EV_CODES.timeupdate, 3.5, 12, 4, 2, "");
    expect(seen.length).toBe(1);
    expect(seen[0]?.currentTime).toBe(3.5);
    expect(seen[0]?.detail.duration).toBe(12);
    expect(seen[0]?.bubbles).toBe(false);
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
    rt.host.mediaById.set(node.__skId, STATE.replace("currentTime=3.5", "currentTime=9"));
    expect(node.currentTime).toBe(9);
  });

  test("a scalar property is one host call for its own value, not the whole state", () => {
    const rt = createRuntime();
    const node = video(rt);
    rt.host.mediaStateReads = 0;
    expect([node.readyState, node.paused, node.loop, node.duration]).toEqual([4, false, true, 12]);
    expect(rt.host.mediaStateReads).toBe(0);
    expect(rt.host.mediaPropReads).toEqual([
      [node.__skId, 5],
      [node.__skId, 2],
      [node.__skId, 10],
      [node.__skId, 1],
    ]);
    expect(node.buffered.length).toBe(1);
    expect(rt.host.mediaStateReads).toBe(1);
    const bare = rt.nodeOps.createElement("video");
    expect([bare.paused, bare.readyState, bare.duration]).toEqual([true, 0, NaN]);
  });

  test("the transport verbs reach __sk_media_call", () => {
    const rt = createRuntime();
    const node = video(rt);
    node.play();
    node.currentTime = 8;
    node.muted = false;
    node.pause();
    expect(rt.host.mediaCalls).toEqual([
      { id: node.__skId, verb: 0, arg: 0 },
      { id: node.__skId, verb: 2, arg: 8 },
      { id: node.__skId, verb: 4, arg: 0 },
      { id: node.__skId, verb: 1, arg: 0 },
    ]);
  });

  test("an errored element reports a MediaError with the advisory text", () => {
    const rt = createRuntime();
    const node = video(rt);
    rt.host.mediaById.set(node.__skId, "error=4\nerrorMessage=this machine has no HEVC decoder; re-encode to H.264");
    expect(node.error?.code).toBe(4);
    expect(node.error?.message).toContain("H.264");
  });

  test("canPlayType is a live probe, and a non-media element has no surface", () => {
    const rt = createRuntime();
    const node = video(rt);
    rt.host.canPlayByType.set('video/mp4; codecs="avc1.640028"', "probably");
    expect(node.canPlayType('video/mp4; codecs="avc1.640028"')).toBe("probably");
    expect(node.canPlayType("application/x-nonsense")).toBe("");
    expect<unknown>(rt.nodeOps.createElement("div").currentTime).toBe(undefined);
  });
});

describe("the listener flag is only written when it changes", () => {
  test("re-patching a handler does not re-announce the bit", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("div");
    rt.patchProp(el, "onClick", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([{ id: el.__skId, code: 1, on: true }]);

    for (let i = 0; i < 10; i++)
      rt.patchProp(
        el,
        "onClick",
        () => {},
        () => {},
      );
    expect(rt.flagsFor(rt.EV_CODES.click).length).toBe(1);
  });

  test("dropping the last handler clears the bit exactly once", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("div");
    rt.patchProp(el, "onClick", null, () => {});
    rt.patchProp(el, "onClick", () => {}, null);
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([
      { id: el.__skId, code: 1, on: true },
      { id: el.__skId, code: 1, on: false },
    ]);
    rt.patchProp(el, "onClick", null, null);
    expect(rt.flagsFor(rt.EV_CODES.click).length).toBe(2);
  });

  test("a second listener for the same code rides the first one's bit", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("div");
    rt.patchProp(el, "onMousedown", null, () => {});
    rt.patchProp(el, "onPointerdown", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.mousedown)).toEqual([{ id: el.__skId, code: 2, on: true }]);
  });

  test("each node keeps its own record", () => {
    const rt = createRuntime();
    const a = rt.nodeOps.createElement("div");
    const b = rt.nodeOps.createElement("div");
    rt.patchProp(a, "onClick", null, () => {});
    rt.patchProp(b, "onClick", null, () => {});
    expect(rt.flagsFor(rt.EV_CODES.click)).toEqual([
      { id: a.__skId, code: 1, on: true },
      { id: b.__skId, code: 1, on: true },
    ]);
  });
});

describe("v-model", () => {
  const bind = (rt: Rt, el: MirrorElement, modifiers: Record<string, boolean>, onUpdate: (v: unknown) => void) =>
    rt.vModelText.created(el, { modifiers }, { props: { "onUpdate:modelValue": onUpdate } });

  test("updates the model on input", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input");
    let model: unknown = null;
    bind(rt, el, {}, (v: unknown) => (model = v));
    rt.host.dispatcher(el.__skId, rt.EV_CODES.input, 0, 0, 0, 0, "typed");
    expect(model).toBe("typed");
    expect(rt.advisories()).toEqual([]);
  });

  test(".lazy still updates the model", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input");
    let model: unknown = null;
    bind(rt, el, { lazy: true }, (v: unknown) => (model = v));
    rt.host.dispatcher(el.__skId, rt.EV_CODES.input, 0, 0, 0, 0, "typed");
    expect(model).toBe("typed");
  });

  test(".number and .trim still apply", () => {
    const rt = createRuntime();
    const el = rt.nodeOps.createElement("input");
    let model: unknown = null;
    bind(rt, el, { number: true, trim: true }, (v: unknown) => (model = v));
    rt.host.dispatcher(el.__skId, rt.EV_CODES.input, 0, 0, 0, 0, "  42  ");
    expect(model).toBe(42);
  });
});
