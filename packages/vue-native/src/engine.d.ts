// What the JS engine provides. tsconfig omits lib.dom on purpose: a name missing here
// must fail to type-check, and the `typeof X === "function"` probes stay meaningful for the
// engine-revision-dependent classes below. Declare a symbol only if redjs really has it.

interface VnConsole {
  log(...args: unknown[]): void;
  warn(...args: unknown[]): void;
  error(...args: unknown[]): void;
  info?(...args: unknown[]): void;
  debug?(...args: unknown[]): void;
}
declare var console: VnConsole;

declare function setTimeout(fn: () => void, ms?: number): number;
declare function clearTimeout(handle: number | undefined): void;
declare function setInterval(fn: () => void, ms?: number): number;
declare function clearInterval(handle: number | undefined): void;
declare var queueMicrotask: (cb: () => void) => void;

/** Runs once per PAINTED frame. Absent in the unit-test harness, so callers probe; hence `var`. */
declare var requestAnimationFrame: ((cb: (t: number) => void) => number) | undefined;
declare var cancelAnimationFrame: ((handle: number) => void) | undefined;

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
  target: unknown;
  currentTarget: unknown;
  readonly eventPhase: number;
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
  removeEventListener(type: string, listener: (ev: VnEvent) => void, options?: boolean | { capture?: boolean }): void;
  dispatchEvent(ev: VnEvent): boolean;
}
declare var EventTarget: {
  new (): VnEventTarget;
  prototype: VnEventTarget;
};

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
  { new (type: string, init?: VnPointerEventInit): VnPointerEvent; prototype: VnPointerEvent } | undefined;

interface VnKeyboardEvent extends VnEvent, Required<VnModifierInit> {
  readonly key: string;
}
declare var KeyboardEvent:
  | {
      new (type: string, init?: VnEventInit & VnModifierInit & { key?: string }): VnKeyboardEvent;
      prototype: VnKeyboardEvent;
    }
  | undefined;

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

declare var FocusEvent: { new (type: string, init?: VnEventInit): VnEvent; prototype: VnEvent } | undefined;

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
      new (type: string, init?: VnEventInit & { propertyName?: string; elapsedTime?: number }): VnTransitionEvent;
      prototype: VnTransitionEvent;
    }
  | undefined;

interface VnAnimationEvent extends VnEvent {
  readonly animationName: string;
  readonly elapsedTime: number;
}
declare var AnimationEvent:
  | {
      new (type: string, init?: VnEventInit & { animationName?: string; elapsedTime?: number }): VnAnimationEvent;
      prototype: VnAnimationEvent;
    }
  | undefined;

/** Real classes with an EMPTY prototype; `runtime.ts` installs the DOM surface on it. `| undefined`: the bun test harness has no `HTMLElement`. */
declare var HTMLElement: { new (): object; prototype: object } | undefined;

/** A property bag with no tree behind it; `runtime.ts` installs the entry points that answer from the mirror. */
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

/** The standard permission surface. `appPermissions` re-broadcasts `change` from it rather than opening a second native subscription. */
interface VnPermissionStatus extends VnEventTarget {
  readonly name: string;
  /** The three-word WEB projection: `"unsupported"` reads as `"denied"`. */
  readonly state: "granted" | "denied" | "prompt";
}
interface VnBatteryManager extends VnEventTarget {
  readonly charging: boolean;
  readonly chargingTime: number;
  readonly dischargingTime: number;
  readonly level: number;
  onchargingchange: ((this: VnBatteryManager, ev: VnEvent) => unknown) | null;
  onchargingtimechange: ((this: VnBatteryManager, ev: VnEvent) => unknown) | null;
  ondischargingtimechange: ((this: VnBatteryManager, ev: VnEvent) => unknown) | null;
  onlevelchange: ((this: VnBatteryManager, ev: VnEvent) => unknown) | null;
}
/** Exposed for branding / instanceof; direct construction throws TypeError. */
declare var BatteryManager: (Function & { readonly prototype: VnBatteryManager }) | undefined;

interface VnNavigator {
  readonly language?: string;
  readonly languages?: readonly string[];
  getBattery?(): Promise<VnBatteryManager>;
  permissions?: {
    query(descriptor: { name: string }): Promise<VnPermissionStatus>;
  };
}
declare var navigator: VnNavigator | undefined;

interface VnFetchResponse {
  readonly ok: boolean;
  readonly status: number;
  json(): Promise<unknown>;
  text?(): Promise<string>;
}
declare var fetch: ((url: string, init?: Record<string, unknown>) => Promise<VnFetchResponse>) | undefined;

interface VnURL {
  readonly href: string;
  readonly protocol: string;
}
interface VnURLConstructor {
  new (url: string, base?: string): VnURL;
  prototype: VnURL;
}
declare var URL: VnURLConstructor;

/**
 * The subset of redjs's Web Audio a native stream source touches. No `declare var AudioContext`:
 * the global is a build property, so code probes the object (`typeof ctx.createRedStreamSource`).
 */
interface VnAudioNode {
  connect(to: VnAudioNode): unknown;
  disconnect(): void;
}

interface VnRedStreamSourceNode extends VnAudioNode {
  readonly streamId: number;
  /** Makes a late native push report NotReady rather than dangle. Probe before use. */
  close?(): void;
}

interface VnRedStreamSourceOptions {
  sampleRate?: number;
  channelCount?: number;
  latencySeconds?: number;
  capacitySeconds?: number;
}

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

type VnNativeAudioProcessorName = "peak-meter-stereo" | "peak-meter-mono" | "stereo-to-mono" | "input-gate";
interface VnNativeAudioWorkletOptions {
  numberOfInputs?: 1;
  numberOfOutputs?: 1;
  outputChannelCount?: [2];
  channelCount?: 2;
  channelCountMode?: "explicit";
  channelInterpretation?: "speakers" | "discrete";
  parameterData?: Record<string, number>;
}
interface VnNativeAudioMessage<T = unknown> extends VnEvent {
  readonly data: T;
}
interface VnNativeAudioWorkletPort extends VnEventTarget {
  onmessage: ((event: VnNativeAudioMessage) => void) | null;
  start(): void;
  close(): void;
}
interface VnNativeAudioWorkletNode extends VnAudioNode {
  readonly processorName: VnNativeAudioProcessorName;
  readonly parameters: ReadonlyMap<string, VnAudioParam>;
  readonly port: VnNativeAudioWorkletPort;
}

interface VnAudioContext {
  loadNativeAudioWorklet(
    name: VnNativeAudioProcessorName,
    options?: VnNativeAudioWorkletOptions,
  ): VnNativeAudioWorkletNode;
  readonly destination: VnAudioNode;
  createRedStreamSource(options?: VnRedStreamSourceOptions): VnRedStreamSourceNode;
  createGain(): VnGainNode;
}
