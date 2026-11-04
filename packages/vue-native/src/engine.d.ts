// What the JS ENGINE provides — and, just as importantly, what it does not.
//
// `tsconfig.json` deliberately omits lib.dom. redjs is not a browser: it ships
// a small, enumerable set of web classes and globals, and this file is that set
// written down. Two things follow from doing it this way rather than pulling in
// `lib.dom.d.ts`:
//
//   1. A name that does not exist here does not type-check. `runtime.ts` is the
//      file that INSTALLS the missing DOM globals (MutationObserver,
//      ResizeObserver, DocumentFragment, getComputedStyle, the tag classes);
//      if lib.dom were loaded, every one of them would already "exist" and the
//      absence they exist to paper over would be invisible to the compiler.
//   2. The `typeof X === "function"` probes the runtime is full of stay
//      MEANINGFUL. Several of these classes are engine-revision-dependent —
//      that is why `synthesize()` guards each constructor and falls back to a
//      `CustomEvent`. They are declared here because the guard needs a name to
//      test, not because they are promised.
//
// Anything genuinely missing from the engine is NOT declared here. If a symbol
// needs adding, the question to answer first is whether redjs really has it.

// ---- diagnostics ----------------------------------------------------------

interface VnConsole {
  log(...args: unknown[]): void;
  warn(...args: unknown[]): void;
  error(...args: unknown[]): void;
  info?(...args: unknown[]): void;
  debug?(...args: unknown[]): void;
}
declare var console: VnConsole;

// ---- timers and frames ----------------------------------------------------

declare function setTimeout(fn: () => void, ms?: number): number;
declare function clearTimeout(handle: number | undefined): void;
declare function setInterval(fn: () => void, ms?: number): number;
declare function clearInterval(handle: number | undefined): void;
/** redjs installs it on the global (web.rs); `var` so `globalThis.queueMicrotask` types too. */
declare var queueMicrotask: (cb: () => void) => void;

/**
 * Driven by the host's frame pump, so a callback runs once per PAINTED frame.
 * Absent in the unit-test harness unless a test supplies one — every caller in
 * `runtime.ts` therefore checks `typeof raf === "function"` first, and the
 * declaration is `var` (not `function`) so that check is legal.
 */
declare var requestAnimationFrame: ((cb: (t: number) => void) => number) | undefined;
declare var cancelAnimationFrame: ((handle: number) => void) | undefined;

// ---- events ---------------------------------------------------------------

interface VnEventInit {
  bubbles?: boolean;
  cancelable?: boolean;
  composed?: boolean;
}

interface VnEvent {
  readonly type: string;
  readonly bubbles: boolean;
  readonly cancelable: boolean;
  readonly defaultPrevented: boolean;
  /** Written by `propagate()` — the mirror node the host targeted. */
  target: unknown;
  /** Rewritten at each step of the propagation walk; `null` once it is over. */
  currentTarget: unknown;
  /** 0 NONE, 1 CAPTURING_PHASE, 2 AT_TARGET, 3 BUBBLING_PHASE — own, per step. */
  readonly eventPhase: number;
  /** The walk's path (target first) during dispatch, `[]` after. */
  composedPath(): unknown[];
  preventDefault(): void;
  stopPropagation(): void;
  stopImmediatePropagation(): void;
}

declare var Event: {
  new (type: string, init?: VnEventInit): VnEvent;
  prototype: VnEvent;
};

interface VnCustomEvent<T = unknown> extends VnEvent {
  readonly detail: T;
}
declare var CustomEvent: {
  new <T = unknown>(type: string, init?: VnEventInit & { detail?: T }): VnCustomEvent<T>;
  prototype: VnCustomEvent;
};

interface VnEventTarget {
  addEventListener(
    type: string,
    listener: (ev: VnEvent) => void,
    options?: boolean | { capture?: boolean; once?: boolean },
  ): void;
  removeEventListener(
    type: string,
    listener: (ev: VnEvent) => void,
    options?: boolean | { capture?: boolean },
  ): void;
  dispatchEvent(ev: VnEvent): boolean;
}
declare var EventTarget: {
  new (): VnEventTarget;
  prototype: VnEventTarget;
};

/** The modifier flags every input event carries. */
interface VnModifierInit {
  ctrlKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
}

interface VnMouseEventInit extends VnEventInit, VnModifierInit {
  clientX?: number;
  clientY?: number;
  button?: number;
  buttons?: number;
  /** UIEventInit's: the click count for the press/release/click family. */
  detail?: number;
}
interface VnMouseEvent extends VnEvent, Required<VnModifierInit> {
  readonly clientX: number;
  readonly clientY: number;
  readonly button: number;
  readonly buttons: number;
  readonly detail: number;
}
declare var MouseEvent: {
  new (type: string, init?: VnMouseEventInit): VnMouseEvent;
  prototype: VnMouseEvent;
};

interface VnPointerEventInit extends VnMouseEventInit {
  pointerId?: number;
  pointerType?: string;
  isPrimary?: boolean;
  width?: number;
  height?: number;
  pressure?: number;
}
interface VnPointerEvent extends VnMouseEvent {
  readonly pointerId: number;
  readonly pointerType: string;
  readonly isPrimary: boolean;
  readonly pressure: number;
}
/** Engine-revision-dependent — probe before constructing. */
declare var PointerEvent:
  | { new (type: string, init?: VnPointerEventInit): VnPointerEvent; prototype: VnPointerEvent }
  | undefined;

interface VnKeyboardEvent extends VnEvent, Required<VnModifierInit> {
  readonly key: string;
}
declare var KeyboardEvent:
  | {
      new (type: string, init?: VnEventInit & VnModifierInit & { key?: string }): VnKeyboardEvent;
      prototype: VnKeyboardEvent;
    }
  | undefined;

/** A MouseEvent, as the UI Events spec has it: `clientX`/`clientY` included. */
interface VnWheelEvent extends VnEvent, Required<VnModifierInit> {
  readonly deltaX: number;
  readonly deltaY: number;
  readonly clientX: number;
  readonly clientY: number;
}
declare var WheelEvent:
  | {
      new (
        type: string,
        init?: VnEventInit & VnModifierInit & { deltaX?: number; deltaY?: number; clientX?: number; clientY?: number },
      ): VnWheelEvent;
      prototype: VnWheelEvent;
    }
  | undefined;

declare var FocusEvent:
  | { new (type: string, init?: VnEventInit): VnEvent; prototype: VnEvent }
  | undefined;

interface VnInputEvent extends VnEvent {
  readonly data: string | null;
}
declare var InputEvent:
  | {
      new (type: string, init?: VnEventInit & { data?: string | null }): VnInputEvent;
      prototype: VnInputEvent;
    }
  | undefined;

interface VnTransitionEvent extends VnEvent {
  readonly propertyName: string;
  readonly elapsedTime: number;
}
declare var TransitionEvent:
  | {
      new (
        type: string,
        init?: VnEventInit & { propertyName?: string; elapsedTime?: number },
      ): VnTransitionEvent;
      prototype: VnTransitionEvent;
    }
  | undefined;

interface VnAnimationEvent extends VnEvent {
  readonly animationName: string;
  readonly elapsedTime: number;
}
declare var AnimationEvent:
  | {
      new (
        type: string,
        init?: VnEventInit & { animationName?: string; elapsedTime?: number },
      ): VnAnimationEvent;
      prototype: VnAnimationEvent;
    }
  | undefined;

// ---- element classes ------------------------------------------------------

/**
 * redjs provides `Element` / `HTMLElement` / `SVGElement` as real classes with
 * an EMPTY prototype — no `getAttribute`, no `closest`, no `style`. The mirror
 * nodes are `Object.create(HTMLElement.prototype)` precisely so that
 * `el instanceof HTMLElement` passes in a reused browser component, and
 * `runtime.ts` hangs the whole DOM surface off that prototype itself.
 *
 * Declared `| undefined` because the bun test harness runs in a world that has
 * no `HTMLElement` at all, which is the branch `ElementProto` guards for.
 */
declare var HTMLElement: { new (): object; prototype: object } | undefined;

// ---- the document ---------------------------------------------------------

/**
 * redjs ships a `document` object with NO TREE BEHIND IT: a property bag with a
 * `head`, a `title`, and a `querySelector` backed by a registry of the elements
 * the engine itself created — which is always empty here, because the retained
 * tree is ours. `runtime.ts` force-installs the entry points that must answer
 * from the mirror instead, so this type says only what may be assumed before
 * that happens.
 */
interface VnDocument {
  title: string;
  head: Record<string, unknown> | undefined;
  body?: unknown;
  documentElement?: unknown;
  addEventListener?: (...args: unknown[]) => void;
  removeEventListener?: (...args: unknown[]) => void;
  [key: string]: unknown;
}
declare var document: VnDocument | undefined;

// ---- navigator ------------------------------------------------------------

/**
 * The STANDARD permission surface, installed by redjs when the host registered
 * a backend. `appPermissions` re-broadcasts `change` from these objects rather
 * than opening a second native subscription, because two drains of one queue
 * would race.
 */
interface VnPermissionStatus extends VnEventTarget {
  readonly name: string;
  /** The three-word WEB projection: `"unsupported"` reads as `"denied"`. */
  readonly state: "granted" | "denied" | "prompt";
}
interface VnNavigator {
  permissions?: {
    query(descriptor: { name: string }): Promise<VnPermissionStatus>;
  };
}
declare var navigator: VnNavigator | undefined;

// ---- fetch ----------------------------------------------------------------

/** Only what the voice shim's TokenSource reads off a response. */
interface VnFetchResponse {
  readonly ok: boolean;
  readonly status: number;
  json(): Promise<unknown>;
  text?(): Promise<string>;
}
declare var fetch:
  | ((url: string, init?: Record<string, unknown>) => Promise<VnFetchResponse>)
  | undefined;

// ---- URL ------------------------------------------------------------------

/**
 * redjs's WHATWG `URL` (vendor/redjs/redjs/src/webapi/url.rs). Only what the
 * link path reads: `href`, the serialised form a browser's `a.href` reports
 * (spaces and non-ASCII percent-encoded), which is what goes to the shell.
 */
interface VnURL {
  readonly href: string;
  readonly protocol: string;
}
/** Named so apps/env.d.ts can merge the statics an app uses
 *  (`createObjectURL` / `revokeObjectURL`) without a second `URL`. */
interface VnURLConstructor {
  new (url: string, base?: string): VnURL;
  prototype: VnURL;
}
/**
 * NOT `| undefined`, unlike the engine-revision classes around it: redjs's
 * `webapi::install` installs `URL` unconditionally (url.rs, no cargo feature),
 * as it does `AbortController`, and the bun harness's world has one too. The
 * optional type was only there for the link path's `typeof URL === "function"`
 * probe, which still compiles and stays as cheap insurance, and it made every
 * app's `URL.createObjectURL(file)` a TS18048 under `strict`.
 */
declare var URL: VnURLConstructor;

// ---- Web Audio ------------------------------------------------------------

/**
 * redjs's Web Audio, as much of it as the voice shim touches and no more.
 *
 * Note what is NOT here: `declare var AudioContext`. `livekit.ts` never reads
 * the global — it only ever receives a context the application already built
 * (`new Room({ webAudioMix: { audioContext } })`, or
 * `remoteAudioTrack.setAudioContext(ctx)`). Whether the global exists at all is
 * a BUILD property: `redjs::webapi::pcm` sits behind vn-app's `audio` cargo
 * feature, and `vn-app --no-default-features` is a legitimate host with no Web
 * Audio at all (vn-voice/CONTRACT.md §5). That is exactly why the shim probes
 * the OBJECT it was handed — `typeof ctx.createRedStreamSource === "function"` —
 * rather than the name, and adding the global here would make that probe look
 * redundant when it is the whole safety story.
 */
interface VnAudioNode {
  connect(to: VnAudioNode): unknown;
  disconnect(): void;
}

/**
 * `RedStreamSourceNode` — redjs's PUSH source (research/reports/m13/webaudio.md
 * §3.2). `streamId` is the handle the native mixer pushes into; it crosses the
 * seam as `OP.SET_PLAYOUT_STREAM`'s only argument, and 0 unroutes.
 */
interface VnRedStreamSourceNode extends VnAudioNode {
  readonly streamId: number;
  /** Makes a late native push report NotReady rather than dangle. Probed
   *  before use — the seam's lifetime rule, and not every revision has it. */
  close?(): void;
}

/**
 * 48 kHz mono, 80 ms: the mixer's own format. `Seam::attach` REFUSES any other
 * channel count rather than play the room at half speed.
 *
 * Every field is OPTIONAL, and the whole argument is: `redjs`'s
 * `create_red_stream_source` defaults all four and has its own test for calling
 * it bare. The voice shim passes all four because it needs the mixer's format
 * exactly, which is a fact about the shim, not about the API.
 */
interface VnRedStreamSourceOptions {
  sampleRate?: number;
  channelCount?: number;
  latencySeconds?: number;
  capacitySeconds?: number;
}

/**
 * One `AudioParam`. `value` is the immediate write; the five scheduling methods
 * are the timeline, and redjs implements all of them — a demo that ramps a gain
 * rather than stepping it is using the same API a browser gives it.
 */
interface VnAudioParam {
  value: number;
  setValueAtTime(value: number, startTime: number): VnAudioParam;
  linearRampToValueAtTime(value: number, endTime: number): VnAudioParam;
  exponentialRampToValueAtTime(value: number, endTime: number): VnAudioParam;
  setTargetAtTime(target: number, startTime: number, timeConstant: number): VnAudioParam;
  cancelScheduledValues(startTime: number): VnAudioParam;
}

interface VnGainNode extends VnAudioNode {
  readonly gain: VnAudioParam;
}

interface VnAudioContext {
  readonly destination: VnAudioNode;
  createRedStreamSource(options?: VnRedStreamSourceOptions): VnRedStreamSourceNode;
  createGain(): VnGainNode;
}
