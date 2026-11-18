// What the JS engine provides, under the web's names. tsconfig omits lib.dom on purpose: a name
// missing here must fail to type-check. Declare a symbol only if redjs really has it.

// `count`, `time` and their kin exist but do nothing, so they are left out.
interface Console {
  log(...args: unknown[]): void;
  info(...args: unknown[]): void;
  warn(...args: unknown[]): void;
  error(...args: unknown[]): void;
  debug(...args: unknown[]): void;
  trace(...args: unknown[]): void;
  assert(condition?: boolean, ...args: unknown[]): void;
  dir(...args: unknown[]): void;
  table(...args: unknown[]): void;
  group(...args: unknown[]): void;
  groupCollapsed(...args: unknown[]): void;
  groupEnd(): void;
  clear(): void;
}
declare var console: Console;

declare function setTimeout(fn: () => void, ms?: number): number;
declare function clearTimeout(handle: number | undefined): void;
declare function setInterval(fn: () => void, ms?: number): number;
declare function clearInterval(handle: number | undefined): void;
declare var queueMicrotask: (cb: () => void) => void;

type FrameRequestCallback = (time: number) => void;
/** Runs once per PAINTED frame. */
declare function requestAnimationFrame(callback: FrameRequestCallback): number;
declare function cancelAnimationFrame(handle: number): void;

interface EventInit {
  bubbles?: boolean;
  cancelable?: boolean;
  composed?: boolean;
}

interface Event {
  readonly type: string;
  readonly bubbles: boolean;
  readonly cancelable: boolean;
  readonly composed: boolean;
  readonly defaultPrevented: boolean;
  readonly isTrusted: boolean;
  readonly timeStamp: number;
  target: unknown;
  currentTarget: unknown;
  readonly eventPhase: number;
  composedPath(): unknown[];
  preventDefault(): void;
  stopPropagation(): void;
  stopImmediatePropagation(): void;
}
declare var Event: {
  new (type: string, eventInitDict?: EventInit): Event;
  prototype: Event;
};

interface CustomEventInit<T = unknown> extends EventInit {
  detail?: T;
}
interface CustomEvent<T = unknown> extends Event {
  readonly detail: T;
}
declare var CustomEvent: {
  new <T = unknown>(type: string, eventInitDict?: CustomEventInit<T>): CustomEvent<T>;
  prototype: CustomEvent;
};

interface EventListenerOptions {
  capture?: boolean;
}
interface AddEventListenerOptions extends EventListenerOptions {
  once?: boolean;
  passive?: boolean;
  signal?: AbortSignal;
}

interface EventListener {
  (evt: Event): void;
}
interface EventListenerObject {
  handleEvent(object: Event): void;
}
type EventListenerOrEventListenerObject = EventListener | EventListenerObject;

interface EventTarget {
  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  removeEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | EventListenerOptions,
  ): void;
  dispatchEvent(event: Event): boolean;
}
declare var EventTarget: {
  new (): EventTarget;
  prototype: EventTarget;
};

interface AbortSignal extends EventTarget {
  readonly aborted: boolean;
  readonly reason: unknown;
  onabort: ((ev: Event) => unknown) | null;
  throwIfAborted(): void;
}
declare var AbortSignal: {
  new (): AbortSignal;
  prototype: AbortSignal;
  abort(reason?: unknown): AbortSignal;
  timeout(milliseconds: number): AbortSignal;
  any(signals: AbortSignal[]): AbortSignal;
};
interface AbortController {
  readonly signal: AbortSignal;
  abort(reason?: unknown): void;
}
declare var AbortController: { new (): AbortController; prototype: AbortController };

interface EventModifierInit {
  ctrlKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
}

interface UIEventInit extends EventInit {
  detail?: number;
}
interface UIEvent extends Event {
  readonly detail: number;
  readonly view: unknown;
  readonly which: number;
}
declare var UIEvent: { new (type: string, eventInitDict?: UIEventInit): UIEvent; prototype: UIEvent };

interface MouseEventInit extends UIEventInit, EventModifierInit {
  screenX?: number;
  screenY?: number;
  clientX?: number;
  clientY?: number;
  movementX?: number;
  movementY?: number;
  button?: number;
  buttons?: number;
  relatedTarget?: unknown;
}
interface MouseEvent extends UIEvent {
  readonly ctrlKey: boolean;
  readonly shiftKey: boolean;
  readonly altKey: boolean;
  readonly metaKey: boolean;
  readonly screenX: number;
  readonly screenY: number;
  readonly clientX: number;
  readonly clientY: number;
  readonly pageX: number;
  readonly pageY: number;
  readonly offsetX: number;
  readonly offsetY: number;
  readonly movementX: number;
  readonly movementY: number;
  readonly x: number;
  readonly y: number;
  readonly button: number;
  readonly buttons: number;
  readonly relatedTarget: unknown;
  getModifierState(keyArg: string): boolean;
}
declare var MouseEvent: {
  new (type: string, eventInitDict?: MouseEventInit): MouseEvent;
  prototype: MouseEvent;
};

interface PointerEventInit extends MouseEventInit {
  pointerId?: number;
  pointerType?: string;
  isPrimary?: boolean;
  width?: number;
  height?: number;
  pressure?: number;
  tangentialPressure?: number;
  tiltX?: number;
  tiltY?: number;
  twist?: number;
  altitudeAngle?: number;
  azimuthAngle?: number;
  coalescedEvents?: PointerEvent[];
  predictedEvents?: PointerEvent[];
}
interface PointerEvent extends MouseEvent {
  readonly pointerId: number;
  readonly pointerType: string;
  readonly isPrimary: boolean;
  readonly width: number;
  readonly height: number;
  readonly pressure: number;
  readonly tangentialPressure: number;
  readonly tiltX: number;
  readonly tiltY: number;
  readonly twist: number;
  readonly altitudeAngle: number;
  readonly azimuthAngle: number;
  getCoalescedEvents(): PointerEvent[];
  getPredictedEvents(): PointerEvent[];
}
declare var PointerEvent: {
  new (type: string, eventInitDict?: PointerEventInit): PointerEvent;
  prototype: PointerEvent;
};

interface KeyboardEventInit extends UIEventInit, EventModifierInit {
  key?: string;
  code?: string;
  location?: number;
  repeat?: boolean;
  isComposing?: boolean;
}
interface KeyboardEvent extends UIEvent {
  readonly ctrlKey: boolean;
  readonly shiftKey: boolean;
  readonly altKey: boolean;
  readonly metaKey: boolean;
  readonly key: string;
  readonly code: string;
  readonly location: number;
  readonly repeat: boolean;
  readonly isComposing: boolean;
  getModifierState(keyArg: string): boolean;
}
declare var KeyboardEvent: {
  new (type: string, eventInitDict?: KeyboardEventInit): KeyboardEvent;
  prototype: KeyboardEvent;
};

interface WheelEventInit extends MouseEventInit {
  deltaX?: number;
  deltaY?: number;
  deltaZ?: number;
  deltaMode?: number;
}
interface WheelEvent extends MouseEvent {
  readonly deltaX: number;
  readonly deltaY: number;
  readonly deltaZ: number;
  readonly deltaMode: number;
}
declare var WheelEvent: { new (type: string, eventInitDict?: WheelEventInit): WheelEvent; prototype: WheelEvent };

interface FocusEventInit extends UIEventInit {
  relatedTarget?: unknown;
}
interface FocusEvent extends UIEvent {
  readonly relatedTarget: unknown;
}
declare var FocusEvent: { new (type: string, eventInitDict?: FocusEventInit): FocusEvent; prototype: FocusEvent };

interface InputEventInit extends UIEventInit {
  data?: string | null;
  inputType?: string;
  isComposing?: boolean;
}
interface InputEvent extends UIEvent {
  readonly data: string | null;
  readonly inputType: string;
  readonly isComposing: boolean;
}
declare var InputEvent: { new (type: string, eventInitDict?: InputEventInit): InputEvent; prototype: InputEvent };

interface TransitionEventInit extends EventInit {
  propertyName?: string;
  elapsedTime?: number;
  pseudoElement?: string;
}
interface TransitionEvent extends Event {
  readonly propertyName: string;
  readonly elapsedTime: number;
  readonly pseudoElement: string;
}
declare var TransitionEvent: {
  new (type: string, eventInitDict?: TransitionEventInit): TransitionEvent;
  prototype: TransitionEvent;
};

interface AnimationEventInit extends EventInit {
  animationName?: string;
  elapsedTime?: number;
  pseudoElement?: string;
}
interface AnimationEvent extends Event {
  readonly animationName: string;
  readonly elapsedTime: number;
  readonly pseudoElement: string;
}
declare var AnimationEvent: {
  new (type: string, eventInitDict?: AnimationEventInit): AnimationEvent;
  prototype: AnimationEvent;
};

// Real classes; `runtime.ts` installs the DOM surface on their prototypes, and env.d.ts types it.
interface Node extends EventTarget {}
interface Element extends Node {}
interface HTMLElement extends Element {}
declare var HTMLElement: { new (): HTMLElement; prototype: HTMLElement };

/** Its tree entry points are installed by `runtime.ts`; env.d.ts types them. */
interface Document extends Node {
  title: string;
}
declare var document: Document;

interface PermissionStatus extends EventTarget {
  readonly name: string;
  /** The three-word WEB projection: `"unsupported"` reads as `"denied"`. */
  readonly state: PermissionState;
  onchange: ((ev: Event) => unknown) | null;
}
type PermissionState = "granted" | "denied" | "prompt";
interface Permissions {
  query(permissionDesc: { name: string }): Promise<PermissionStatus>;
}

interface BatteryManager extends EventTarget {
  readonly charging: boolean;
  readonly chargingTime: number;
  readonly dischargingTime: number;
  readonly level: number;
  onchargingchange: ((this: BatteryManager, ev: Event) => unknown) | null;
  onchargingtimechange: ((this: BatteryManager, ev: Event) => unknown) | null;
  ondischargingtimechange: ((this: BatteryManager, ev: Event) => unknown) | null;
  onlevelchange: ((this: BatteryManager, ev: Event) => unknown) | null;
}
/** Exposed for branding / instanceof; direct construction throws TypeError. */
declare var BatteryManager: (Function & { readonly prototype: BatteryManager }) | undefined;

interface Navigator {
  readonly language: string;
  readonly languages: readonly string[];
  /** Only with the `battery` feature; probe before calling. */
  getBattery?(): Promise<BatteryManager>;
  readonly permissions: Permissions;
  /** The system media controls and media keys; Windows and macOS only, so probe before use. */
  readonly mediaSession?: MediaSession;
}
declare var navigator: Navigator;

interface MediaImage {
  src: string;
  sizes?: string;
  type?: string;
}
interface MediaMetadataInit {
  title?: string;
  artist?: string;
  album?: string;
  artwork?: MediaImage[];
}
interface MediaMetadata {
  title: string;
  artist: string;
  album: string;
  artwork: readonly MediaImage[];
}
declare var MediaMetadata: { new (init?: MediaMetadataInit): MediaMetadata; prototype: MediaMetadata };
type MediaSessionAction =
  | "play"
  | "pause"
  | "seekbackward"
  | "seekforward"
  | "previoustrack"
  | "nexttrack"
  | "skipad"
  | "stop"
  | "seekto"
  | "togglemicrophone"
  | "togglecamera"
  | "togglescreenshare"
  | "hangup"
  | "previousslide"
  | "nextslide"
  | "enterpictureinpicture"
  | "voiceactivity";
interface MediaSessionActionDetails {
  action: MediaSessionAction;
  seekTime?: number;
  seekOffset?: number;
  fastSeek?: boolean;
}
interface MediaPositionState {
  duration?: number;
  playbackRate?: number;
  position?: number;
}
interface MediaSession {
  metadata: MediaMetadata | null;
  playbackState: "none" | "paused" | "playing";
  setActionHandler(action: MediaSessionAction, handler: ((details: MediaSessionActionDetails) => void) | null): void;
  setPositionState(state?: MediaPositionState): void;
}

type BlobPart = Blob | ArrayBuffer | ArrayBufferView | string;
type BufferSource = ArrayBufferView | ArrayBuffer;
interface BlobPropertyBag {
  type?: string;
  endings?: "transparent" | "native";
}
interface FilePropertyBag extends BlobPropertyBag {
  lastModified?: number;
}
interface ReadableStreamReadResult<T> {
  done: boolean;
  value?: T;
}
interface ReadableStreamDefaultReader<R = unknown> {
  read(): Promise<ReadableStreamReadResult<R>>;
  cancel(reason?: unknown): Promise<void>;
  releaseLock(): void;
}
interface ReadableStream<R = unknown> extends AsyncIterable<R> {
  getReader(): ReadableStreamDefaultReader<R>;
  cancel(reason?: unknown): Promise<void>;
  readonly locked: boolean;
}
interface Blob {
  readonly size: number;
  readonly type: string;
  arrayBuffer(): Promise<ArrayBuffer>;
  bytes(): Promise<Uint8Array>;
  text(): Promise<string>;
  slice(start?: number, end?: number, contentType?: string): Blob;
  stream(): ReadableStream<Uint8Array>;
}
declare var Blob: { new (blobParts?: Iterable<BlobPart>, options?: BlobPropertyBag): Blob; prototype: Blob };

interface File extends Blob {
  readonly name: string;
  readonly lastModified: number;
  readonly lastModifiedDate: Date;
  readonly webkitRelativePath: string;
}
declare var File: {
  new (fileBits: Iterable<BlobPart>, fileName: string, options?: FilePropertyBag): File;
  prototype: File;
};

interface Headers {
  get(name: string): string | null;
  has(name: string): boolean;
}
interface Response {
  readonly ok: boolean;
  readonly status: number;
  readonly statusText: string;
  readonly url: string;
  readonly redirected: boolean;
  readonly headers: Headers;
  json(): Promise<unknown>;
  text(): Promise<string>;
  arrayBuffer(): Promise<ArrayBuffer>;
  blob(): Promise<Blob>;
}
type RequestInit = Record<string, unknown>;
declare function fetch(input: string | URL, init?: RequestInit): Promise<Response>;

interface URL {
  href: string;
  readonly origin: string;
  protocol: string;
  username: string;
  password: string;
  host: string;
  hostname: string;
  port: string;
  pathname: string;
  search: string;
  readonly searchParams: URLSearchParams;
  hash: string;
  toString(): string;
  toJSON(): string;
}
declare var URL: {
  new (url: string | URL, base?: string | URL): URL;
  prototype: URL;
  canParse(url: string | URL, base?: string | URL): boolean;
  createObjectURL(obj: Blob): string;
  revokeObjectURL(url: string): void;
};

interface URLSearchParams {
  readonly size: number;
  append(name: string, value: string): void;
  delete(name: string): void;
  get(name: string): string | null;
  getAll(name: string): string[];
  has(name: string): boolean;
  set(name: string, value: string): void;
  sort(): void;
  forEach(callbackfn: (value: string, key: string, parent: URLSearchParams) => void, thisArg?: unknown): void;
  entries(): IterableIterator<[string, string]>;
  keys(): IterableIterator<string>;
  values(): IterableIterator<string>;
  [Symbol.iterator](): IterableIterator<[string, string]>;
  toString(): string;
}
declare var URLSearchParams: {
  new (init?: string | Iterable<readonly [string, string]> | Record<string, string> | URLSearchParams): URLSearchParams;
  prototype: URLSearchParams;
};

// Web Audio.

interface AudioParam {
  value: number;
  setValueAtTime(value: number, startTime: number): AudioParam;
  linearRampToValueAtTime(value: number, endTime: number): AudioParam;
  exponentialRampToValueAtTime(value: number, endTime: number): AudioParam;
  setTargetAtTime(target: number, startTime: number, timeConstant: number): AudioParam;
  cancelScheduledValues(cancelTime: number): AudioParam;
}

type ChannelCountMode = "max" | "clamped-max" | "explicit";
type ChannelInterpretation = "speakers" | "discrete";

interface AudioNode {
  readonly numberOfInputs: number;
  readonly numberOfOutputs: number;
  channelCount: number;
  channelCountMode: ChannelCountMode;
  channelInterpretation: ChannelInterpretation;
  connect(destinationNode: AudioNode, output?: number, input?: number): AudioNode;
  disconnect(outputOrDestination?: number | AudioNode, output?: number, input?: number): void;
}

interface AudioDestinationNode extends AudioNode {
  /** The output device's channels; `channelCount` may be raised up to it. */
  readonly maxChannelCount: number;
}

interface StereoPannerNode extends AudioNode {
  readonly pan: AudioParam;
}

/** `"spatial"` is Skathi's: the panner renders as a Windows spatial audio object while one is free. Not web API. */
type PanningModelType = "equalpower" | "HRTF" | "spatial";
type DistanceModelType = "linear" | "inverse" | "exponential";

interface PannerNode extends AudioNode {
  panningModel: PanningModelType;
  distanceModel: DistanceModelType;
  readonly positionX: AudioParam;
  readonly positionY: AudioParam;
  readonly positionZ: AudioParam;
  readonly orientationX: AudioParam;
  readonly orientationY: AudioParam;
  readonly orientationZ: AudioParam;
  refDistance: number;
  maxDistance: number;
  rolloffFactor: number;
  coneInnerAngle: number;
  coneOuterAngle: number;
  coneOuterGain: number;
}

interface AudioListener {
  readonly positionX: AudioParam;
  readonly positionY: AudioParam;
  readonly positionZ: AudioParam;
  readonly forwardX: AudioParam;
  readonly forwardY: AudioParam;
  readonly forwardZ: AudioParam;
  readonly upX: AudioParam;
  readonly upY: AudioParam;
  readonly upZ: AudioParam;
}

/** `AudioContext.spatialAudio`: whether `"spatial"` panners render as spatial audio objects. Not web API. */
interface SkSpatialAudioStatus {
  /** A spatial renderer has the context's output. */
  readonly active: boolean;
  /** A panner of the context asked for `"spatial"`. */
  readonly requested: boolean;
  /** `"windows-sonic"`, `"dolby-atmos-headphones"`, `"dts-headphone-x"`, …; `null` while inactive. */
  readonly format: string | null;
  readonly maxObjects: number;
  readonly objectsInUse: number;
  /** Why a requested spatial output is not active; its panners render with HRTF meanwhile. */
  readonly reason: string | null;
}

interface GainNode extends AudioNode {
  readonly gain: AudioParam;
}

interface OscillatorNode extends AudioNode {
  type: string;
  readonly frequency: AudioParam;
  readonly detune: AudioParam;
  onended: (() => void) | null;
  start(when?: number): void;
  stop(when?: number): void;
}

interface AnalyserNode extends AudioNode {
  fftSize: number;
  smoothingTimeConstant: number;
  minDecibels: number;
  maxDecibels: number;
  readonly frequencyBinCount: number;
  getByteFrequencyData(array: Uint8Array): void;
  getFloatFrequencyData(array: Float32Array): void;
  getByteTimeDomainData(array: Uint8Array): void;
  getFloatTimeDomainData(array: Float32Array): void;
}

interface AudioBuffer {
  readonly numberOfChannels: number;
  readonly length: number;
  readonly sampleRate: number;
  readonly duration: number;
  getChannelData(channel: number): Float32Array;
  copyToChannel(source: Float32Array, channelNumber: number): void;
}

interface AudioBufferSourceNode extends AudioNode {
  buffer: AudioBuffer | null;
  loop: boolean;
  loopStart: number;
  loopEnd: number;
  onended: (() => void) | null;
  start(when?: number, offset?: number, duration?: number): void;
  stop(when?: number): void;
}

interface AudioNodeOptions {
  channelCount?: number;
  channelCountMode?: ChannelCountMode;
  channelInterpretation?: ChannelInterpretation;
}
interface ChannelSplitterOptions extends AudioNodeOptions {
  numberOfOutputs?: number;
}
interface ChannelMergerOptions extends AudioNodeOptions {
  numberOfInputs?: number;
}
interface ChannelSplitterNode extends AudioNode {}
interface ChannelMergerNode extends AudioNode {}
declare var ChannelSplitterNode: {
  new (context: AudioContext, options?: ChannelSplitterOptions): ChannelSplitterNode;
  prototype: ChannelSplitterNode;
};
declare var ChannelMergerNode: {
  new (context: AudioContext, options?: ChannelMergerOptions): ChannelMergerNode;
  prototype: ChannelMergerNode;
};

/** A native stream source: blocks pushed from JS or Rust play without a worklet. Not web API. */
interface SkRedStreamSourceNode extends AudioNode {
  readonly streamId: number;
  push(block: Float32Array): number;
  readonly queuedSeconds?: number;
  readonly ready: boolean;
  readonly underflowed: boolean;
  readonly overflowed: boolean;
  /** Makes a late native push report NotReady rather than dangle. Probe before use. */
  close?(): void;
}

interface SkRedStreamSourceOptions {
  sampleRate?: number;
  channelCount?: number;
  latencySeconds?: number;
  capacitySeconds?: number;
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
interface SkNativeAudioMessage<T = unknown> extends Event {
  readonly data: T;
}
interface SkNativeAudioWorkletPort extends EventTarget {
  onmessage: ((event: SkNativeAudioMessage) => void) | null;
  start(): void;
  close(): void;
}
interface SkNativeAudioWorkletNode extends AudioNode {
  readonly processorName: SkNativeAudioProcessorName;
  readonly parameters: ReadonlyMap<string, AudioParam>;
  readonly port: SkNativeAudioWorkletPort;
}

interface AudioContext {
  readonly state: "suspended" | "running" | "closed";
  readonly sampleRate: number;
  readonly currentTime: number;
  readonly destination: AudioDestinationNode;
  readonly listener: AudioListener;
  readonly audioWorklet?: unknown;
  /** Skathi's spatial output status; present with the `spatial-audio` feature. Not web API. */
  readonly spatialAudio?: SkSpatialAudioStatus;
  createGain(): GainNode;
  createOscillator(): OscillatorNode;
  createAnalyser(): AnalyserNode;
  createPanner(): PannerNode;
  createStereoPanner(): StereoPannerNode;
  createBuffer(numberOfChannels: number, length: number, sampleRate: number): AudioBuffer;
  createBufferSource(): AudioBufferSourceNode;
  createChannelSplitter(numberOfOutputs?: number): ChannelSplitterNode;
  createChannelMerger(numberOfInputs?: number): ChannelMergerNode;
  decodeAudioData(audioData: ArrayBuffer): Promise<AudioBuffer>;
  resume(): Promise<void>;
  suspend(): Promise<void>;
  close(): Promise<void>;
  /** Skathi's native stream source; not web API. */
  createRedStreamSource(options?: SkRedStreamSourceOptions): SkRedStreamSourceNode;
  /** Skathi's built-in native processors; not web API. */
  loadNativeAudioWorklet(
    name: SkNativeAudioProcessorName,
    options?: SkNativeAudioWorkletOptions,
  ): SkNativeAudioWorkletNode;
}
declare var AudioContext: { new (): AudioContext; prototype: AudioContext };
/** Declared so a probe compiles; there is no offline rendering. */
declare var OfflineAudioContext: undefined;
