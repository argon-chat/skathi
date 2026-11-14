// What the JS engine provides. tsconfig omits lib.dom on purpose: a name missing here
// must fail to type-check, and the `typeof X === "function"` probes stay meaningful for the
// engine-revision-dependent classes below. Declare a symbol only if redjs really has it.

interface SkConsole {
  log(...args: unknown[]): void;
  warn(...args: unknown[]): void;
  error(...args: unknown[]): void;
  info?(...args: unknown[]): void;
  debug?(...args: unknown[]): void;
}
declare var console: SkConsole;

declare function setTimeout(fn: () => void, ms?: number): number;
declare function clearTimeout(handle: number | undefined): void;
declare function setInterval(fn: () => void, ms?: number): number;
declare function clearInterval(handle: number | undefined): void;
declare var queueMicrotask: (cb: () => void) => void;

/** Runs once per PAINTED frame. Absent in the unit-test harness, so callers probe; hence `var`. */
declare var requestAnimationFrame: ((cb: (t: number) => void) => number) | undefined;
declare var cancelAnimationFrame: ((handle: number) => void) | undefined;

interface SkEventInit {
  bubbles?: boolean;
  cancelable?: boolean;
  composed?: boolean;
}

interface SkEvent {
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
  new (type: string, init?: SkEventInit): SkEvent;
  prototype: SkEvent;
};

interface SkCustomEvent<T = unknown> extends SkEvent {
  readonly detail: T;
}
declare var CustomEvent: {
  new <T = unknown>(type: string, init?: SkEventInit & { detail?: T }): SkCustomEvent<T>;
  prototype: SkCustomEvent;
};

interface SkEventTarget {
  addEventListener(
    type: string,
    listener: (ev: SkEvent) => void,
    options?: boolean | { capture?: boolean; once?: boolean },
  ): void;
  removeEventListener(type: string, listener: (ev: SkEvent) => void, options?: boolean | { capture?: boolean }): void;
  dispatchEvent(ev: SkEvent): boolean;
}
declare var EventTarget: {
  new (): SkEventTarget;
  prototype: SkEventTarget;
};

interface SkModifierInit {
  ctrlKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
}

interface SkMouseEventInit extends SkEventInit, SkModifierInit {
  clientX?: number;
  clientY?: number;
  button?: number;
  buttons?: number;
  detail?: number;
}
interface SkMouseEvent extends SkEvent, Required<SkModifierInit> {
  readonly clientX: number;
  readonly clientY: number;
  readonly button: number;
  readonly buttons: number;
  readonly detail: number;
}
declare var MouseEvent: {
  new (type: string, init?: SkMouseEventInit): SkMouseEvent;
  prototype: SkMouseEvent;
};

interface SkPointerEventInit extends SkMouseEventInit {
  pointerId?: number;
  pointerType?: string;
  isPrimary?: boolean;
  width?: number;
  height?: number;
  pressure?: number;
}
interface SkPointerEvent extends SkMouseEvent {
  readonly pointerId: number;
  readonly pointerType: string;
  readonly isPrimary: boolean;
  readonly pressure: number;
}
/** Engine-revision-dependent — probe before constructing. */
declare var PointerEvent:
  { new (type: string, init?: SkPointerEventInit): SkPointerEvent; prototype: SkPointerEvent } | undefined;

interface SkKeyboardEvent extends SkEvent, Required<SkModifierInit> {
  readonly key: string;
}
declare var KeyboardEvent:
  | {
      new (type: string, init?: SkEventInit & SkModifierInit & { key?: string }): SkKeyboardEvent;
      prototype: SkKeyboardEvent;
    }
  | undefined;

interface SkWheelEvent extends SkEvent, Required<SkModifierInit> {
  readonly deltaX: number;
  readonly deltaY: number;
  readonly clientX: number;
  readonly clientY: number;
}
declare var WheelEvent:
  | {
      new (
        type: string,
        init?: SkEventInit & SkModifierInit & { deltaX?: number; deltaY?: number; clientX?: number; clientY?: number },
      ): SkWheelEvent;
      prototype: SkWheelEvent;
    }
  | undefined;

declare var FocusEvent: { new (type: string, init?: SkEventInit): SkEvent; prototype: SkEvent } | undefined;

interface SkInputEvent extends SkEvent {
  readonly data: string | null;
}
declare var InputEvent:
  | {
      new (type: string, init?: SkEventInit & { data?: string | null }): SkInputEvent;
      prototype: SkInputEvent;
    }
  | undefined;

interface SkTransitionEvent extends SkEvent {
  readonly propertyName: string;
  readonly elapsedTime: number;
}
declare var TransitionEvent:
  | {
      new (type: string, init?: SkEventInit & { propertyName?: string; elapsedTime?: number }): SkTransitionEvent;
      prototype: SkTransitionEvent;
    }
  | undefined;

interface SkAnimationEvent extends SkEvent {
  readonly animationName: string;
  readonly elapsedTime: number;
}
declare var AnimationEvent:
  | {
      new (type: string, init?: SkEventInit & { animationName?: string; elapsedTime?: number }): SkAnimationEvent;
      prototype: SkAnimationEvent;
    }
  | undefined;

/** Real classes with an EMPTY prototype; `runtime.ts` installs the DOM surface on it. `| undefined`: the bun test harness has no `HTMLElement`. */
declare var HTMLElement: { new (): object; prototype: object } | undefined;

/** A property bag with no tree behind it; `runtime.ts` installs the entry points that answer from the mirror. */
interface SkDocument {
  title: string;
  head: Record<string, unknown> | undefined;
  body?: unknown;
  documentElement?: unknown;
  addEventListener?: (...args: unknown[]) => void;
  removeEventListener?: (...args: unknown[]) => void;
  [key: string]: unknown;
}
declare var document: SkDocument | undefined;

/** The standard permission surface. `appPermissions` re-broadcasts `change` from it rather than opening a second native subscription. */
interface SkPermissionStatus extends SkEventTarget {
  readonly name: string;
  /** The three-word WEB projection: `"unsupported"` reads as `"denied"`. */
  readonly state: "granted" | "denied" | "prompt";
}
interface SkBatteryManager extends SkEventTarget {
  readonly charging: boolean;
  readonly chargingTime: number;
  readonly dischargingTime: number;
  readonly level: number;
  onchargingchange: ((this: SkBatteryManager, ev: SkEvent) => unknown) | null;
  onchargingtimechange: ((this: SkBatteryManager, ev: SkEvent) => unknown) | null;
  ondischargingtimechange: ((this: SkBatteryManager, ev: SkEvent) => unknown) | null;
  onlevelchange: ((this: SkBatteryManager, ev: SkEvent) => unknown) | null;
}
/** Exposed for branding / instanceof; direct construction throws TypeError. */
declare var BatteryManager: (Function & { readonly prototype: SkBatteryManager }) | undefined;

interface SkNavigator {
  readonly language?: string;
  readonly languages?: readonly string[];
  getBattery?(): Promise<SkBatteryManager>;
  permissions?: {
    query(descriptor: { name: string }): Promise<SkPermissionStatus>;
  };
}
declare var navigator: SkNavigator | undefined;

interface SkFetchResponse {
  readonly ok: boolean;
  readonly status: number;
  json(): Promise<unknown>;
  text?(): Promise<string>;
}
declare var fetch: ((url: string, init?: Record<string, unknown>) => Promise<SkFetchResponse>) | undefined;

interface SkURL {
  readonly href: string;
  readonly protocol: string;
}
interface SkURLConstructor {
  new (url: string, base?: string): SkURL;
  prototype: SkURL;
}
declare var URL: SkURLConstructor;

/**
 * The subset of redjs's Web Audio a native stream source touches. No `declare var AudioContext`:
 * the global is a build property, so code probes the object (`typeof ctx.createRedStreamSource`).
 */
interface SkAudioNode {
  connect(to: SkAudioNode): unknown;
  disconnect(): void;
}

interface SkRedStreamSourceNode extends SkAudioNode {
  readonly streamId: number;
  /** Makes a late native push report NotReady rather than dangle. Probe before use. */
  close?(): void;
}

interface SkRedStreamSourceOptions {
  sampleRate?: number;
  channelCount?: number;
  latencySeconds?: number;
  capacitySeconds?: number;
}

interface SkAudioParam {
  value: number;
  setValueAtTime(value: number, startTime: number): SkAudioParam;
  linearRampToValueAtTime(value: number, endTime: number): SkAudioParam;
  exponentialRampToValueAtTime(value: number, endTime: number): SkAudioParam;
  setTargetAtTime(target: number, startTime: number, timeConstant: number): SkAudioParam;
  cancelScheduledValues(startTime: number): SkAudioParam;
}

interface SkGainNode extends SkAudioNode {
  readonly gain: SkAudioParam;
}

type SkNativeAudioProcessorName = "peak-meter-stereo" | "peak-meter-mono" | "stereo-to-mono" | "input-gate";
interface SkNativeAudioWorkletOptions {
  numberOfInputs?: 1;
  numberOfOutputs?: 1;
  outputChannelCount?: [2];
  channelCount?: 2;
  channelCountMode?: "explicit";
  channelInterpretation?: "speakers" | "discrete";
  parameterData?: Record<string, number>;
}
interface SkNativeAudioMessage<T = unknown> extends SkEvent {
  readonly data: T;
}
interface SkNativeAudioWorkletPort extends SkEventTarget {
  onmessage: ((event: SkNativeAudioMessage) => void) | null;
  start(): void;
  close(): void;
}
interface SkNativeAudioWorkletNode extends SkAudioNode {
  readonly processorName: SkNativeAudioProcessorName;
  readonly parameters: ReadonlyMap<string, SkAudioParam>;
  readonly port: SkNativeAudioWorkletPort;
}

interface SkAudioContext {
  loadNativeAudioWorklet(
    name: SkNativeAudioProcessorName,
    options?: SkNativeAudioWorkletOptions,
  ): SkNativeAudioWorkletNode;
  readonly destination: SkAudioNode;
  createRedStreamSource(options?: SkRedStreamSourceOptions): SkRedStreamSourceNode;
  createGain(): SkGainNode;
}
