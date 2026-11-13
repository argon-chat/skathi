import type { MirrorElement, MirrorNode, MirrorTimeRanges } from "./types";
import { defineDom, defineDomGet } from "./dom-members";
import { warnCompat } from "../compat";
import { decodeMessage, encodeMessage } from "../iframe";
import { isMirrorNode } from "./listeners";
import { branded } from "./collections";
import { patchProp } from "./patch-prop";
import { isMirrorEl, ownExpando } from "./controls";
import { propagateAtGlobal } from "./events";

// HTMLMediaElement: media state lives in the HOST (`currentTime` moves in the OS pipeline), so accessors read
// `__vn_media_state` live, uncached; verbs go through `__vn_media_call`.
const MEDIA_CALL = {
  play: 0,
  pause: 1,
  currentTime: 2,
  volume: 3,
  muted: 4,
  playbackRate: 5,
  loop: 6,
} as const satisfies Record<string, VnMediaVerb>;
const MEDIA_TAGS = new Set<string>(["VIDEO", "AUDIO"]);
const mediaStreams = new WeakMap<MirrorElement, unknown>();
defineDomGet(
  "srcObject",
  function (this: MirrorElement) {
    return isMediaEl(this) ? (mediaStreams.get(this) ?? null) : undefined;
  },
  function (this: MirrorElement, value: unknown) {
    if (!isMediaEl(this)) return;
    if (value == null) {
      mediaStreams.delete(this);
      if (this.getAttribute("src")?.startsWith("vn-stream://")) this.removeAttribute("src");
      return;
    }
    const stream = value as { getVideoTracks?: () => { __vnVideoSrc?: string }[] };
    const track = typeof stream.getVideoTracks === "function" ? stream.getVideoTracks()[0] : undefined;
    if (!track?.__vnVideoSrc?.startsWith("vn-stream://")) {
      // An <audio> given an audio stream (kept alive while Web Audio plays it) holds the stream and plays nothing.
      if (this.tagName === "AUDIO") {
        mediaStreams.set(this, value);
        if (this.getAttribute("src")?.startsWith("vn-stream://")) this.removeAttribute("src");
        return;
      }
      throw new TypeError("video.srcObject requires a native video MediaStream");
    }
    mediaStreams.set(this, value);
    this.setAttribute("src", track.__vnVideoSrc);
  },
);
const isMediaEl = (n: MirrorNode | null | undefined): n is MirrorElement =>
  !!n && MEDIA_TAGS.has((n as MirrorElement).tagName);
function mediaState(node: MirrorNode | null | undefined): Record<string, string> | null {
  if (!isMediaEl(node) || !node.__vnId) return null;
  const raw = __vn_media_state(node.__vnId);
  if (!raw) return null;
  const out: Record<string, string> = {};
  for (const line of raw.split("\n")) {
    if (!line) continue;
    const eq = line.indexOf("=");
    if (eq > 0) out[line.slice(0, eq)] = line.slice(eq + 1);
  }
  return out;
}
const mediaCall = (node: MirrorNode, verb: VnMediaVerb, arg = 0): boolean =>
  isMediaEl(node) && node.__vnId ? __vn_media_call(node.__vnId, verb, arg) : false;
function timeRanges(raw: string | undefined): MirrorTimeRanges {
  const list = raw
    ? raw
        .split(",")
        .filter(Boolean)
        .map((pair) => {
          const [start = NaN, end = NaN] = pair.split(":").map(Number);
          return { start, end };
        })
    : [];
  return {
    length: list.length,
    start: (i: number) => list[i]?.start ?? NaN,
    end: (i: number) => list[i]?.end ?? NaN,
    ranges: list,
  };
}
function defineMediaGet(
  name: string,
  read: (s: Record<string, string>) => unknown,
  write?: (value: never) => void,
): void {
  defineDomGet(
    name,
    function (this: MirrorElement) {
      const s = mediaState(this);
      return s ? read(s) : isMediaEl(this) ? read({}) : undefined;
    },
    write,
  );
}
const num = (s: Record<string, string>, k: string, fallback: number): number =>
  s[k] === undefined ? fallback : Number(s[k]);
const bool = (s: Record<string, string>, k: string, fallback: boolean): boolean =>
  s[k] === undefined ? fallback : s[k] === "true";

/** A SCALAR media property: one host call (`__vn_media_prop`), since the whole state string costs ~0.2 ms per read on Media Foundation. */
function defineMediaScalar(
  name: string,
  prop: VnMediaProp,
  fallback: number | boolean,
  write?: (value: never) => void,
): void {
  const isBool = typeof fallback === "boolean";
  defineDomGet(
    name,
    function (this: MirrorElement) {
      if (!isMediaEl(this)) return undefined;
      if (typeof __vn_media_prop === "function") {
        const v = this.__vnId ? __vn_media_prop(this.__vnId, prop) : undefined;
        if (v === undefined) return fallback;
        return isBool ? v !== 0 : v;
      }
      const s = mediaState(this) ?? {};
      return isBool ? bool(s, name, fallback as boolean) : num(s, name, fallback as number);
    },
    write,
  );
}

defineMediaScalar("currentTime", 0, 0, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.currentTime, Number(v));
});
defineMediaScalar("duration", 1, NaN);
defineMediaScalar("paused", 2, true);
defineMediaScalar("ended", 3, false);
defineMediaScalar("seeking", 4, false);
defineMediaScalar("readyState", 5, 0);
defineMediaScalar("networkState", 6, 0);
defineMediaScalar("volume", 7, 1, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.volume, Number(v));
});
defineMediaScalar("muted", 8, false, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.muted, v ? 1 : 0);
});
defineMediaScalar("playbackRate", 9, 1, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.playbackRate, Number(v));
});
defineMediaScalar("loop", 10, false, function (this: MirrorElement, v: unknown) {
  mediaCall(this, MEDIA_CALL.loop, v ? 1 : 0);
});
defineMediaScalar("videoWidth", 11, 0);
defineMediaScalar("videoHeight", 12, 0);
defineMediaGet("buffered", (s) => timeRanges(s.buffered));
defineMediaGet("seekable", (s) => timeRanges(s.seekable));
defineMediaGet("error", (s) => {
  const code = num(s, "error", 0);
  return code ? { code, message: s.errorMessage ?? "" } : null;
});

defineDom("play", function (this: MirrorElement) {
  mediaCall(this, MEDIA_CALL.play);
  // Resolves immediately (the host's play is synchronous), so `el.play().catch(…)` does not throw.
  return Promise.resolve();
});
defineDom("pause", function (this: MirrorElement) {
  mediaCall(this, MEDIA_CALL.pause);
});
defineDom("load", function (this: MirrorElement) {
  // The host loads off the `src` prop, edge-triggered; there is no load verb.
  if (isMediaEl(this)) {
    warnCompat(
      "media-load",
      "video.load() is a no-op — the host loads from the `src` prop, edge-triggered; " +
        "to force a reload, clear `src` and set it again",
    );
  }
});
defineDom("canPlayType", function (this: MirrorElement, type: unknown) {
  if (!isMediaEl(this) || !this.__vnId) return "";
  return __vn_media_can_play(this.__vnId, String(type));
});

// HTMLIFrameElement: the document lives in a host-owned platform webview, so it is cross-origin: post to it and hear
// from it. `contentWindow` is one object per element, so `event.source === iframe.contentWindow` holds.
export const isFrameEl = (x: unknown): x is MirrorElement => isMirrorEl(x) && x.tag === "iframe";
const frameWindows = new WeakMap<MirrorElement, object>();
const frameOf = new WeakMap<object, MirrorElement>();

function frameTargetOrigin(arg: unknown): string {
  if (arg == null) return "/"; // the WindowPostMessageOptions default: same origin only
  if (typeof arg === "object") {
    const t = (arg as { targetOrigin?: unknown; transfer?: unknown[] }).targetOrigin;
    if ((arg as { transfer?: unknown[] }).transfer?.length) {
      throw new DOMException(
        "Failed to execute 'postMessage': transferring is not supported for an <iframe>.",
        "DataCloneError",
      );
    }
    return t == null ? "/" : String(t);
  }
  return String(arg);
}

function frameWindow(el: MirrorElement): object {
  let win = frameWindows.get(el);
  if (win) return win;
  // No prototype: nothing of this realm is reachable through another origin's window handle.
  win = Object.freeze(
    Object.assign(Object.create(null) as Record<string, unknown>, {
      postMessage(message: unknown, target?: unknown, transfer?: unknown[]) {
        if (arguments.length === 0)
          throw new TypeError("Failed to execute 'postMessage': 1 argument required, but only 0 present.");
        if (transfer?.length) {
          throw new DOMException(
            "Failed to execute 'postMessage': transferring is not supported for an <iframe>.",
            "DataCloneError",
          );
        }
        const origin = frameTargetOrigin(target);
        // Encoded now, in the sender, so a value that cannot cross throws HERE.
        const wire = JSON.stringify(encodeMessage(message, (o) => isMirrorNode(o)));
        if (!el.__vnId || !el.isConnected || typeof __vn_iframe_post !== "function") return;
        __vn_iframe_post(el.__vnId, wire, origin);
      },
      toString: () => "[object Window]",
    }),
  );
  frameWindows.set(el, win);
  frameOf.set(win, el);
  return win;
}

defineDomGet("contentWindow", function (this: MirrorElement) {
  if (!isFrameEl(this)) return undefined;
  return this.isConnected ? frameWindow(this) : null;
});
defineDomGet("contentDocument", function (this: MirrorElement) {
  return isFrameEl(this) ? null : undefined;
});
defineDom("getSVGDocument", function (this: MirrorElement) {
  return null;
});

defineDomGet(
  "sandbox",
  function (this: MirrorElement) {
    if (!isFrameEl(this)) return undefined;
    const node = this;
    const parts = () => (node.attrs.sandbox ?? "").split(/\s+/).filter(Boolean);
    const write = (tokens: string[]) => node.setAttribute("sandbox", tokens.join(" "));
    const list = {
      get value() {
        return node.attrs.sandbox ?? "";
      },
      set value(v: string) {
        node.setAttribute("sandbox", String(v));
      },
      add(...names: string[]) {
        const now = parts();
        for (const n of names) if (!now.includes(n)) now.push(n);
        write(now);
      },
      remove(...names: string[]) {
        write(parts().filter((p) => !names.includes(p)));
      },
      toggle(name: string, force?: boolean) {
        const on = force === undefined ? !parts().includes(name) : !!force;
        if (on) list.add(name);
        else list.remove(name);
        return on;
      },
      replace(from: string, to: string) {
        const now = parts();
        const i = now.indexOf(from);
        if (i < 0) return false;
        now[i] = to;
        write([...new Set(now)]);
        return true;
      },
      contains: (name: string) => parts().includes(name),
      supports: (name: string) => FRAME_SANDBOX_TOKENS.has(String(name).toLowerCase()),
      get length() {
        return parts().length;
      },
      item: (i: number) => parts()[i] ?? null,
      forEach(fn: (token: string, index: number) => void) {
        parts().forEach((t, i) => fn(t, i));
      },
      toString: () => node.attrs.sandbox ?? "",
      [Symbol.iterator]: () => parts()[Symbol.iterator](),
    };
    Object.defineProperty(this, "sandbox", {
      get: () => list,
      set: (v: unknown) => node.setAttribute("sandbox", String(v)),
      configurable: true,
    });
    return branded("DOMTokenList", list);
  },
  function (this: MirrorElement, v: unknown) {
    if (!isFrameEl(this)) return ownExpando(this, "sandbox", v);
    this.setAttribute("sandbox", String(v));
  } as (value: never) => void,
);
const FRAME_SANDBOX_TOKENS = new Set([
  "allow-downloads",
  "allow-forms",
  "allow-modals",
  "allow-orientation-lock",
  "allow-pointer-lock",
  "allow-popups",
  "allow-popups-to-escape-sandbox",
  "allow-presentation",
  "allow-same-origin",
  "allow-scripts",
  "allow-top-navigation",
  "allow-top-navigation-by-user-activation",
  "allow-top-navigation-to-custom-protocols",
]);

export function deliverFrameMessage(target: MirrorNode, key: string | undefined): boolean {
  if (!isFrameEl(target) || key == null) return true;
  let origin = "null";
  let data: unknown;
  try {
    const parsed = JSON.parse(key) as { o?: string; d?: unknown };
    origin = String(parsed.o ?? "null");
    data = decodeMessage(parsed.d);
  } catch (e) {
    console.error("[vue-native] an <iframe> message that could not be decoded was dropped:", e);
    return true;
  }
  const source = frameWindow(target);
  const init = { data, origin, source, lastEventId: "", ports: [] as unknown[] };
  let ev: VnEvent;
  const Ctor = (globalThis as { MessageEvent?: new (type: string, init: object) => VnEvent }).MessageEvent;
  try {
    // A real MessageEvent refuses a foreign `source`, so the fields are defined on it below.
    ev = Ctor ? new Ctor("message", { data, origin, lastEventId: "" }) : (new Event("message") as unknown as VnEvent);
  } catch {
    ev = new Event("message") as unknown as VnEvent;
  }
  for (const k of Object.keys(init) as (keyof typeof init)[]) {
    Object.defineProperty(ev, k, { value: init[k], configurable: true, enumerable: true });
  }
  propagateAtGlobal(globalThis, "message", ev);
  return true;
}

/** `vnFeatures`: host capabilities a page should check; `iframe` is false without a webview backend (Linux, or Windows without WebView2), where every `src` answers `error`. */
Object.defineProperty(globalThis, "vnFeatures", {
  value: Object.freeze(
    Object.defineProperty({}, "iframe", {
      get: () => typeof __vn_iframe_supported === "function" && __vn_iframe_supported(),
      enumerable: true,
    }),
  ),
  configurable: true,
});

// HTMLImageElement: the host answers every `src` with `load` or `error`; the dispatcher records it on the element
// (`_img`) before the event runs, and discards one for a `src` the element has since dropped.
const isImgEl = (x: unknown): x is MirrorElement => isMirrorEl(x) && x.tag === "img";
const imgAnswer = (el: MirrorElement) => (el._img && el._img.src === el.attrs.src ? el._img : undefined);
const SRC_TAGS = new Set(["img", "video", "audio", "source", "iframe", "lottie"]);

defineDomGet(
  "src",
  function (this: MirrorElement) {
    if (!isMirrorEl(this) || !SRC_TAGS.has(this.tag)) return undefined;
    return this.attrs.src ?? "";
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this) || !SRC_TAGS.has(this.tag)) return ownExpando(this, "src", v);
    patchProp(this, "src", undefined, String(v));
  } as (value: never) => void,
);
defineDomGet("currentSrc", function (this: MirrorElement) {
  if (!isImgEl(this)) return undefined;
  return imgAnswer(this)?.ok ? this.attrs.src : "";
});
defineDomGet("complete", function (this: MirrorElement) {
  if (!isImgEl(this)) return undefined;
  const src = this.attrs.src;
  return src == null || src === "" || !!imgAnswer(this);
});
defineDomGet("naturalWidth", function (this: MirrorElement) {
  if (!isImgEl(this)) return undefined;
  return imgAnswer(this)?.w ?? 0;
});
defineDomGet("naturalHeight", function (this: MirrorElement) {
  if (!isImgEl(this)) return undefined;
  return imgAnswer(this)?.h ?? 0;
});
defineDom("decode", function (this: MirrorElement) {
  if (!isImgEl(this)) return Promise.resolve();
  const el = this;
  const settled = (): Promise<void> | null => {
    const answer = imgAnswer(el);
    if (answer?.ok) return Promise.resolve();
    if (answer || !el.attrs.src)
      return Promise.reject(new DOMException("The source image cannot be decoded.", "EncodingError"));
    return null;
  };
  return (
    settled() ??
    new Promise<void>((resolve, reject) => {
      const done = () => {
        el.removeEventListener("load", done);
        el.removeEventListener("error", done);
        (settled() ?? Promise.reject(new DOMException("The source image cannot be decoded.", "EncodingError"))).then(
          resolve,
          reject,
        );
      };
      el.addEventListener("load", done);
      el.addEventListener("error", done);
    })
  );
});
