// `packages/runtime/src/engine.d.ts` is deliberately not this file. NOTHING IS DECLARED THAT
// THE HOST DOES NOT PROVIDE.

export {};

// the window, as a global EventTarget — crates/sk-app/src/window.prelude.js The prelude installs
// these on `globalThis` before the app bundle evaluates.

declare global {
  function getSelection(): { toString(): string } | null;
  function addEventListener(
    type: string,
    listener: (ev: SkEvent) => void,
    options?: boolean | { capture?: boolean; once?: boolean },
  ): void;
  function removeEventListener(
    type: string,
    listener: (ev: SkEvent) => void,
    options?: boolean | { capture?: boolean },
  ): void;
  function dispatchEvent(ev: SkEvent): boolean;

  var innerWidth: number;
  var innerHeight: number;
  var outerWidth: number;
  var outerHeight: number;
  var screenX: number;
  var screenY: number;
  var devicePixelRatio: number;

  function resizeBy(dw: number, dh: number): void;

  function open(url?: string, target?: string, features?: string): null;

  interface SkLaunchParams {
    readonly targetURL: string | null;
    readonly files: readonly (SkFileSystemFileHandle | SkFileSystemDirectoryHandle)[];
  }
  var launchQueue: {
    setConsumer(consumer: (params: SkLaunchParams) => unknown): void;
  };

  var screen: {
    readonly width: number;
    readonly height: number;
    readonly availWidth: number;
    readonly availHeight: number;
    readonly availLeft: number;
    readonly availTop: number;
    readonly isExtended: boolean;
  };

  /** A query the evaluator cannot run advises rather than answering false forever. */
  function matchMedia(query: string): SkMediaQueryList;

  interface SkMediaQueryList extends SkEventTarget {
    readonly matches: boolean;
    readonly media: string;
  }

  var performance: { now(): number };

  var ononline: ((ev: SkEvent) => unknown) | null;
  var onoffline: ((ev: SkEvent) => unknown) | null;
}

declare global {
  interface SkNavigator {
    windowControlsOverlay: SkWindowControlsOverlay;

    /**
     * The async Clipboard API — vendor/redjs/redjs/src/webapi/clipboard.rs, over the host's own
     * clipboard (the same one the native `<input>` uses for Ctrl+C: one owner, not two).
     */
    clipboard: {
      readText(): Promise<string>;
      writeText(text: string): Promise<void>;
      read(): Promise<SkClipboardItem[]>;
      write(items: SkClipboardItem[]): Promise<void>;
    };

    /** `el.click()` and `dispatchEvent` never activate. */
    readonly userActivation: {
      readonly isActive: boolean;
      readonly hasBeenActive: boolean;
    };

    readonly onLine: boolean;
    readonly platform: string;
    readonly maxTouchPoints: number;
  }

  interface SkWindowControlsOverlay extends SkEventTarget {
    readonly visible: boolean;
    getTitlebarAreaRect(): SkWindowRect;
  }

  interface SkWindowChrome {
    titleBarStyle: string;
    displayMode: string;
    fallbackTitleBar: boolean;
    overlayVisible: boolean;
    hostControls: boolean;
    client: { width: number; height: number };
    contentInset: { left: number; top: number; right: number; bottom: number };
  }
}

declare global {
  interface SkDocument {
    readonly fullscreenElement: SkElement | null;
    exitFullscreen(): Promise<void>;
    // NOT `documentElement`: engine.d.ts already declares it as `documentElement?: unknown`, and
    // an interface merge cannot narrow a property that is already there.

    /**
     * Inert, and returns `false` — exactly as a browser does when a command cannot be executed.
     * clipboard.rs advises once naming `navigator.clipboard`.
     */
    execCommand(command: string): boolean;
  }
}

declare global {
  interface SkDomRect {
    readonly x: number;
    readonly y: number;
    readonly top: number;
    readonly left: number;
    readonly right: number;
    readonly bottom: number;
    readonly width: number;
    readonly height: number;
  }

  interface SkScrollToOptions {
    left?: number;
    top?: number;
    behavior?: "auto" | "instant" | "smooth";
  }

  interface SkScrollIntoViewOptions {
    block?: "start" | "center" | "end" | "nearest";
    inline?: "start" | "center" | "end" | "nearest";
    behavior?: "auto" | "instant" | "smooth";
  }

  interface SkElement extends SkEventTarget {
    /** The `id` ATTRIBUTE; the host's node id is not reachable from script. */
    id: string;
    getBoundingClientRect(): SkDomRect;
    /**
     * `0` on an element that does not scroll, and a write to one does nothing, exactly as in a
     * browser.
     */
    scrollTop: number;
    scrollLeft: number;
    readonly scrollWidth: number;
    readonly scrollHeight: number;
    scrollTo(x?: number | SkScrollToOptions, y?: number): void;
    scrollBy(x?: number | SkScrollToOptions, y?: number): void;
    scrollIntoView(arg?: boolean | SkScrollIntoViewOptions): void;
    focus(): void;
    blur(): void;
    click(): void;
    requestFullscreen(): Promise<void>;
  }

  interface SkMediaElement extends SkElement {
    currentTime: number;
    readonly duration: number;
    readonly paused: boolean;
    readonly ended: boolean;
    readonly seeking: boolean;
    readonly readyState: number;
    volume: number;
    muted: boolean;
    playbackRate: number;
    loop: boolean;
    readonly videoWidth: number;
    readonly videoHeight: number;
    /** A real `MediaError`: the message is the advisory naming what THIS machine cannot decode. */
    readonly error: SkMediaError | null;
    play(): Promise<void>;
    pause(): void;
    canPlayType(type: string): string;
  }

  interface SkMediaError {
    readonly code: number;
    readonly message: string;
  }

  interface SkMediaEvent extends SkEvent {
    readonly currentTime: number;
    readonly duration: number;
    readonly readyState: number;
    readonly playbackRate: number;
  }
  interface SkLottieLoopEvent extends SkEvent {
    readonly currentTime: number;
    readonly totalTime: number;
    readonly currentLoop: number;
  }
}

declare global {
  interface SkStorage {
    readonly length: number;
    key(index: number): string | null;
    getItem(key: string): string | null;
    /** Typed strictly so the coercion is written down at the call site rather than being invisible. */
    setItem(key: string, value: string): void;
    removeItem(key: string): void;
    clear(): void;
  }
  var localStorage: SkStorage;
  /** Per-run, never written anywhere. */
  var sessionStorage: SkStorage;
}

declare global {
  interface SkFetchResponse {
    readonly url: string;
    readonly redirected: boolean;
    readonly headers: { get(name: string): string | null };
    arrayBuffer(): Promise<ArrayBuffer>;
    blob(): Promise<SkBlob>;
  }

  /**
   * So unlike `fetch` and `AudioContext` next door this one is not `| undefined`: a redjs that
   * lacks it does not exist, and apps/devtools' request timeout depends on it.
   */
  interface SkAbortSignal extends SkEventTarget {
    readonly aborted: boolean;
    readonly reason: unknown;
    throwIfAborted(): void;
  }
  interface SkAbortController {
    readonly signal: SkAbortSignal;
    abort(reason?: unknown): void;
  }
  var AbortController: { new (): SkAbortController; prototype: SkAbortController };
}

declare global {
  type SkBlobPart = SkBlob | ArrayBuffer | ArrayBufferView | string;
  interface SkBlobPropertyBag {
    type?: string;
    endings?: "transparent" | "native";
  }
  interface SkByteStream extends AsyncIterable<Uint8Array> {
    getReader(): {
      read(): Promise<{ done: boolean; value?: Uint8Array }>;
      cancel(reason?: unknown): Promise<void>;
      releaseLock(): void;
    };
    cancel(reason?: unknown): Promise<void>;
    readonly locked: boolean;
  }
  interface SkBlob {
    readonly size: number;
    readonly type: string;
    arrayBuffer(): Promise<ArrayBuffer>;
    bytes(): Promise<Uint8Array>;
    text(): Promise<string>;
    slice(start?: number, end?: number, contentType?: string): SkBlob;
    stream(): SkByteStream;
  }
  var Blob: { new (parts?: Iterable<SkBlobPart>, options?: SkBlobPropertyBag): SkBlob; prototype: SkBlob };

  interface SkFile extends SkBlob {
    readonly name: string;
    readonly lastModified: number;
    readonly lastModifiedDate: Date;
    readonly webkitRelativePath: string;
  }
  var File: {
    new (bits: Iterable<SkBlobPart>, name: string, options?: SkBlobPropertyBag & { lastModified?: number }): SkFile;
    prototype: SkFile;
  };

  interface SkFileList extends Iterable<SkFile> {
    readonly length: number;
    item(index: number): SkFile | null;
    readonly [index: number]: SkFile;
  }
  var FileList: { new (): SkFileList; prototype: SkFileList };

  interface SkURLConstructor {
    createObjectURL(blob: SkBlob): string;
    revokeObjectURL(url: string): void;
  }

  interface SkDataTransferItem {
    readonly kind: "string" | "file" | "";
    readonly type: string;
    getAsString(callback: ((data: string) => void) | null): void;
    getAsFile(): SkFile | null;
    getAsFileSystemHandle?(): Promise<unknown | null>;
  }
  interface SkDataTransferItemList extends Iterable<SkDataTransferItem> {
    readonly length: number;
    readonly [index: number]: SkDataTransferItem;
    add(data: string, type: string): SkDataTransferItem | null;
    add(file: SkFile): SkDataTransferItem | null;
    remove(index: number): void;
    clear(): void;
  }
  interface SkDataTransfer {
    dropEffect: "none" | "copy" | "link" | "move";
    effectAllowed: "none" | "copy" | "copyLink" | "copyMove" | "link" | "linkMove" | "move" | "all" | "uninitialized";
    readonly items: SkDataTransferItemList;
    readonly types: readonly string[];
    readonly files: SkFileList;
    getData(format: string): string;
    setData(format: string, data: string): void;
    clearData(format?: string): void;
    setDragImage(image: unknown, x: number, y: number): void;
  }
  var DataTransfer: { new (): SkDataTransfer; prototype: SkDataTransfer };

  interface SkFileInputElement extends SkElement {
    files: SkFileList;
    value: string;
    accept: string;
    multiple: boolean;
    webkitdirectory: boolean;
    /** Needs a user activation (NotAllowedError otherwise). */
    showPicker(): void;
  }

  interface SkCloseEvent extends SkEvent {
    readonly code: number;
    readonly reason: string;
    readonly wasClean: boolean;
  }
  interface SkMessageEvent extends SkEvent {
    readonly data: string | SkBlob | ArrayBuffer;
  }

  interface SkWebSocket extends SkEventTarget {
    readonly readyState: number;
    readonly bufferedAmount: number;
    readonly protocol: string;
    readonly url: string;
    binaryType: "blob" | "arraybuffer";
    onopen: ((ev: SkEvent) => void) | null;
    onerror: ((ev: SkEvent) => void) | null;
    onclose: ((ev: SkCloseEvent) => void) | null;
    onmessage: ((ev: SkMessageEvent) => void) | null;
    send(data: string | ArrayBuffer | ArrayBufferView | SkBlob): void;
    close(code?: number, reason?: string): void;
  }
  var WebSocket: {
    new (url: string, protocols?: string | string[]): SkWebSocket;
    prototype: SkWebSocket;
    readonly CONNECTING: 0;
    readonly OPEN: 1;
    readonly CLOSING: 2;
    readonly CLOSED: 3;
  };
}

// Every picker needs a user activation, else SecurityError.

declare global {
  type SkPermissionState = "granted" | "prompt";
  interface SkFileSystemHandlePermissionDescriptor {
    mode?: "read" | "readwrite";
  }
  interface SkFileSystemHandle {
    readonly kind: "file" | "directory";
    readonly name: string;
    isSameEntry(other: SkFileSystemHandle): Promise<boolean>;
    queryPermission(descriptor?: SkFileSystemHandlePermissionDescriptor): Promise<SkPermissionState>;
    requestPermission(descriptor?: SkFileSystemHandlePermissionDescriptor): Promise<SkPermissionState>;
  }
  interface SkFileSystemFileHandle extends SkFileSystemHandle {
    readonly kind: "file";
    getFile(): Promise<SkFile>;
    createWritable(options?: { keepExistingData?: boolean }): Promise<SkFileSystemWritableFileStream>;
  }
  interface SkFileSystemDirectoryHandle
    extends SkFileSystemHandle, AsyncIterable<[string, SkFileSystemFileHandle | SkFileSystemDirectoryHandle]> {
    readonly kind: "directory";
    getFileHandle(name: string, options?: { create?: boolean }): Promise<SkFileSystemFileHandle>;
    getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<SkFileSystemDirectoryHandle>;
    removeEntry(name: string, options?: { recursive?: boolean }): Promise<void>;
    resolve(possibleDescendant: SkFileSystemHandle): Promise<string[] | null>;
    entries(): AsyncIterableIterator<[string, SkFileSystemFileHandle | SkFileSystemDirectoryHandle]>;
    keys(): AsyncIterableIterator<string>;
    values(): AsyncIterableIterator<SkFileSystemFileHandle | SkFileSystemDirectoryHandle>;
  }
  type SkFileSystemWriteData = SkBlob | ArrayBuffer | ArrayBufferView | string;
  type SkFileSystemWriteChunk =
    | SkFileSystemWriteData
    | { type: "write"; position?: number | null; data: SkFileSystemWriteData }
    | { type: "seek"; position: number }
    | { type: "truncate"; size: number };
  interface SkFileSystemWritableFileStream {
    readonly locked: boolean;
    write(data: SkFileSystemWriteChunk): Promise<void>;
    seek(position: number): Promise<void>;
    truncate(size: number): Promise<void>;
    close(): Promise<void>;
    abort(reason?: unknown): Promise<void>;
  }
  // The construct signatures are lib.dom's: calling one throws "Illegal constructor", but without
  // it `x instanceof FileSystemFileHandle` does not type-check.
  var FileSystemHandle: { new (): SkFileSystemHandle; prototype: SkFileSystemHandle };
  var FileSystemFileHandle: { new (): SkFileSystemFileHandle; prototype: SkFileSystemFileHandle };
  var FileSystemDirectoryHandle: { new (): SkFileSystemDirectoryHandle; prototype: SkFileSystemDirectoryHandle };
  var FileSystemWritableFileStream: {
    new (): SkFileSystemWritableFileStream;
    prototype: SkFileSystemWritableFileStream;
  };

  type SkWellKnownDirectory = "desktop" | "documents" | "downloads" | "music" | "pictures" | "videos";
  interface SkFilePickerAcceptType {
    description?: string;
    accept: Record<string, string | string[]>;
  }
  interface SkFilePickerOptions {
    id?: string;
    startIn?: SkWellKnownDirectory | SkFileSystemHandle;
    types?: SkFilePickerAcceptType[];
    excludeAcceptAllOption?: boolean;
  }
  interface SkOpenFilePickerOptions extends SkFilePickerOptions {
    multiple?: boolean;
  }
  interface SkSaveFilePickerOptions extends SkFilePickerOptions {
    suggestedName?: string;
  }
  interface SkDirectoryPickerOptions {
    id?: string;
    startIn?: SkWellKnownDirectory | SkFileSystemHandle;
    mode?: "read" | "readwrite";
  }
  function showOpenFilePicker(options?: SkOpenFilePickerOptions): Promise<SkFileSystemFileHandle[]>;
  function showSaveFilePicker(options?: SkSaveFilePickerOptions): Promise<SkFileSystemFileHandle>;
  function showDirectoryPicker(options?: SkDirectoryPickerOptions): Promise<SkFileSystemDirectoryHandle>;

  interface SkProgressEvent extends SkEvent {
    readonly lengthComputable: boolean;
    readonly loaded: number;
    readonly total: number;
  }
  var ProgressEvent: {
    new (
      type: string,
      init?: SkEventInit & { lengthComputable?: boolean; loaded?: number; total?: number },
    ): SkProgressEvent;
    prototype: SkProgressEvent;
  };

  interface SkFileReader extends SkEventTarget {
    readonly readyState: 0 | 1 | 2;
    readonly result: string | ArrayBuffer | null;
    readonly error: SkDOMException | null;
    readonly EMPTY: 0;
    readonly LOADING: 1;
    readonly DONE: 2;
    readAsArrayBuffer(blob: SkBlob): void;
    readAsBinaryString(blob: SkBlob): void;
    /** `encoding`, else the Blob type's charset, else UTF-8; a BOM wins over all three. */
    readAsText(blob: SkBlob, encoding?: string): void;
    readAsDataURL(blob: SkBlob): void;
    abort(): void;
    onloadstart: ((ev: SkProgressEvent) => void) | null;
    onprogress: ((ev: SkProgressEvent) => void) | null;
    onload: ((ev: SkProgressEvent) => void) | null;
    onerror: ((ev: SkProgressEvent) => void) | null;
    onabort: ((ev: SkProgressEvent) => void) | null;
    onloadend: ((ev: SkProgressEvent) => void) | null;
  }
  var FileReader: {
    new (): SkFileReader;
    prototype: SkFileReader;
    readonly EMPTY: 0;
    readonly LOADING: 1;
    readonly DONE: 2;
  };
}

declare global {
  interface SkAudioContext {
    readonly state: "suspended" | "running" | "closed";
    readonly sampleRate: number;
    readonly currentTime: number;
    readonly audioWorklet?: unknown;
    createOscillator(): SkOscillatorNode;
    createAnalyser(): SkAnalyserNode;
    createBuffer(channels: number, frames: number, sampleRate: number): SkAudioBuffer;
    createBufferSource(): SkAudioBufferSourceNode;
    createChannelSplitter(numberOfOutputs?: number): SkChannelSplitterNode;
    createChannelMerger(numberOfInputs?: number): SkChannelMergerNode;
    decodeAudioData(bytes: ArrayBuffer): Promise<SkAudioBuffer>;
    resume(): Promise<void>;
    suspend(): Promise<void>;
    createRedStreamSource(options?: Partial<SkRedStreamSourceOptions>): SkRedStreamSourceNode;
  }

  interface SkRedStreamSourceNode {
    push(block: Float32Array): number;
    readonly queuedSeconds?: number;
    readonly ready: boolean;
    readonly underflowed: boolean;
    readonly overflowed: boolean;
  }

  /**
   * NOT a merge onto engine.d.ts's `SkGainNode`, which already declares `readonly gain: { value:
   * number }` — the `.value` half — and a merge cannot narrow a property that is already there.
   */
  interface SkGainNodeWithParam extends SkAudioNode {
    readonly gain: SkAudioParam;
  }

  interface SkAudioParam {
    value: number;
    cancelScheduledValues(when: number): void;
    setValueAtTime(value: number, when: number): void;
    linearRampToValueAtTime(value: number, when: number): void;
    exponentialRampToValueAtTime(value: number, when: number): void;
  }

  interface SkOscillatorNode extends SkAudioNode {
    type: string;
    readonly frequency: SkAudioParam;
    readonly detune: SkAudioParam;
    onended: (() => void) | null;
    start(when?: number): void;
    stop(when?: number): void;
  }

  interface SkAnalyserNode extends SkAudioNode {
    fftSize: number;
    smoothingTimeConstant: number;
    readonly frequencyBinCount: number;
    getByteFrequencyData(into: Uint8Array): void;
    getFloatTimeDomainData(into: Float32Array): void;
  }

  interface SkAudioBuffer {
    readonly numberOfChannels: number;
    readonly length: number;
    readonly sampleRate: number;
    readonly duration: number;
    getChannelData(channel: number): Float32Array;
    copyToChannel(source: Float32Array, channel: number): void;
  }

  interface SkAudioBufferSourceNode extends SkAudioNode {
    buffer: SkAudioBuffer | null;
    onended: (() => void) | null;
    start(when?: number): void;
    stop(when?: number): void;
  }

  interface SkAudioNode {
    readonly numberOfInputs: number;
    readonly numberOfOutputs: number;
    connect(to: SkAudioNode, output?: number, input?: number): unknown;
    disconnect(outputOrDestination?: number | SkAudioNode, output?: number, input?: number): void;
  }

  interface SkChannelSplitterNode extends SkAudioNode {
    channelCount: number;
    channelCountMode: string;
    channelInterpretation: string;
  }

  interface SkChannelMergerNode extends SkAudioNode {
    channelCount: number;
    channelCountMode: string;
    channelInterpretation: string;
  }

  interface SkChannelNodeOptions {
    channelCount?: number;
    channelCountMode?: string;
    channelInterpretation?: string;
  }

  var ChannelSplitterNode: {
    new (context: SkAudioContext, options?: SkChannelNodeOptions & { numberOfOutputs?: number }): SkChannelSplitterNode;
    prototype: SkChannelSplitterNode;
  };
  var ChannelMergerNode: {
    new (context: SkAudioContext, options?: SkChannelNodeOptions & { numberOfInputs?: number }): SkChannelMergerNode;
    prototype: SkChannelMergerNode;
  };

  var AudioContext: { new (): SkAudioContext; prototype: SkAudioContext } | undefined;
  /** Declared so the probe compiles; `undefined` is the honest type. */
  var OfflineAudioContext: undefined;
}

declare global {
  /**
   * It is for an app's own adaptations to the host, `if (globalThis.isSk) …`, without importing
   * "@skathi-native/runtime" (which a browser build of the same app cannot resolve).
   */
  var isSk: true;

  var __SK_SCENARIO__: string | undefined;

  /**
   * `SK_HTTP_ORIGIN` — where MediaPage points its http(s) `src` demos, so the page can be aimed at
   * a real server without an internet dependency.
   */
  var __SK_HTTP_ORIGIN__: string | undefined;

  var __sk_perf_mark: ((name: string) => void) | undefined;

  /**
   * The scenario driver reaches VoicePage's verbs through `ctl.ts eval`, so the page publishes
   * them under a known name.
   */
  var __voice: Record<string, (...args: never[]) => unknown> | undefined;
}

// The DOM redjs's mirror tree answers to (packages/runtime/src/runtime.ts). `document` itself stays
// engine.d.ts's loose `SkDocument | undefined`; a caller that needs the methods narrows it once, to SkDomDocument.
declare global {
  var window: typeof globalThis;
  var onerror: ((...args: unknown[]) => unknown) | null;
  function atob(data: string): string;
  function getComputedStyle(el: SkElement): { readonly fontSize: string };

  type RequestInit = Record<string, unknown>;

  type Event = SkEvent;
  type CustomEvent<T = unknown> = SkCustomEvent<T>;
  type EventTarget = SkEventTarget;
  type MouseEvent = SkMouseEvent;
  type KeyboardEvent = SkKeyboardEvent;
  type WheelEvent = SkWheelEvent;
  type FocusEvent = SkFocusEvent;
  type CompositionEvent = SkCompositionEvent;
  type ErrorEvent = SkErrorEvent;
  type Blob = SkBlob;
  type File = SkFile;
  type DOMException = SkDOMException;
  type FileSystemFileHandle = SkFileSystemFileHandle;
  type DOMRect = SkDomRect;
  type Node = SkNode;
  type Element = SkElement;
  type HTMLElement = SkElement;
  type Document = SkDomDocument;
  type HTMLInputElement = SkInputElement;
  type HTMLTextAreaElement = SkTextAreaElement;
  type HTMLSelectElement = SkSelectElement;
  type HTMLImageElement = SkImageElement;
  type HTMLCanvasElement = SkCanvasElement;
  type HTMLVideoElement = SkMediaElement;
  type HTMLIFrameElement = SkIframeElement;
  type MediaStreamTrack = SkMediaStreamTrack;
  type MediaStreamConstraints = SkMediaStreamConstraints;
  type MediaDeviceInfo = SkMediaDeviceInfo;

  interface SkEventTarget {
    addEventListener(
      type: "keydown" | "keyup",
      listener: (ev: SkKeyboardEvent) => unknown,
      options?: boolean | { capture?: boolean; once?: boolean },
    ): void;
    addEventListener(
      type: "focusin" | "focusout",
      listener: (ev: SkFocusEvent) => unknown,
      options?: boolean | { capture?: boolean; once?: boolean },
    ): void;
    removeEventListener(
      type: "keydown" | "keyup",
      listener: (ev: SkKeyboardEvent) => unknown,
      options?: boolean | { capture?: boolean },
    ): void;
    removeEventListener(
      type: "focusin" | "focusout",
      listener: (ev: SkFocusEvent) => unknown,
      options?: boolean | { capture?: boolean },
    ): void;
  }
  function addEventListener(
    type: "keydown" | "keyup",
    listener: (ev: SkKeyboardEvent) => unknown,
    options?: boolean | { capture?: boolean; once?: boolean },
  ): void;
  function removeEventListener(
    type: "keydown" | "keyup",
    listener: (ev: SkKeyboardEvent) => unknown,
    options?: boolean | { capture?: boolean },
  ): void;

  interface SkFocusEvent extends SkEvent {
    readonly relatedTarget: unknown;
  }
  interface SkCompositionEvent extends SkEvent {
    readonly data: string;
  }
  interface SkErrorEvent extends SkEvent {
    readonly message: string;
  }

  interface SkNode extends SkEventTarget {
    readonly nodeType: number;
    readonly nodeName: string;
    readonly isConnected: boolean;
    readonly parentNode: SkNode | null;
    readonly firstChild: SkNode | null;
    readonly lastChild: SkNode | null;
    readonly childNodes: SkNodeList;
    readonly ownerDocument: SkDomDocument;
    textContent: string | null;
    appendChild<T extends SkNode>(child: T): T;
    removeChild<T extends SkNode>(child: T): T;
    contains(other: SkNode | null): boolean;
  }

  interface SkInlineStyle {
    fontSize: string;
    border: string;
    display: string;
    position: string;
    left: string;
    top: string;
    width: string;
    height: string;
    pointerEvents: string;
  }

  interface SkDomRectList extends Iterable<SkDomRect> {
    readonly length: number;
    readonly [index: number]: SkDomRect;
  }

  interface SkNodeList extends Iterable<SkNode> {
    readonly length: number;
    readonly [index: number]: SkNode;
    item(index: number): SkNode | null;
  }

  interface SkElementList<E extends SkElement = SkElement> extends Iterable<E> {
    readonly length: number;
    readonly [index: number]: E;
    item(index: number): E | null;
    namedItem(name: string): E | null;
  }

  interface SkAnimationOptions {
    duration?: number;
    easing?: string;
  }

  interface SkElement extends SkNode {
    readonly tagName: string;
    readonly namespaceURI: string | null;
    readonly classList: object;
    readonly dataset: object;
    className: string;
    readonly style: SkInlineStyle;
    readonly parentElement: SkElement | null;
    readonly offsetWidth: number;
    readonly offsetHeight: number;
    readonly clientWidth: number;
    readonly clientHeight: number;
    readonly children: SkElementList;
    getClientRects(): SkDomRectList;
    getElementsByClassName(names: string): SkElementList;
    getAttribute(name: string): string | null;
    setAttribute(name: string, value: string): void;
    removeAttribute(name: string): void;
    closest(selector: string): SkElement | null;
    querySelector<E extends SkElement = SkElement>(selector: string): E | null;
    querySelectorAll<E extends SkElement = SkElement>(selector: string): SkElementList<E>;
    remove(): void;
    focus(options?: { preventScroll?: boolean }): void;
    animate(keyframes: Record<string, string>[], options?: SkAnimationOptions): unknown;
  }

  interface SkInputElement extends SkElement {
    value: string;
    readonly files: SkFileList | null;
  }
  interface SkTextAreaElement extends SkElement {
    value: string;
  }
  interface SkSelectElement extends SkElement {
    value: string;
  }
  interface SkImageElement extends SkElement {
    src: string;
    readonly complete: boolean;
    readonly naturalWidth: number;
    readonly naturalHeight: number;
    onload: ((ev: SkEvent) => unknown) | null;
  }

  interface SkImageData {
    readonly data: Uint8ClampedArray;
  }
  interface SkCanvas2D {
    fillStyle: string;
    fillRect(x: number, y: number, w: number, h: number): void;
    createImageData(w: number, h: number): SkImageData;
    putImageData(data: SkImageData, x: number, y: number): void;
  }
  interface SkCanvasElement extends SkElement {
    width: number;
    height: number;
    getContext(kind: "2d"): SkCanvas2D | null;
  }

  interface SkTokenList {
    value: string;
    readonly length: number;
    contains(token: string): boolean;
    add(...tokens: string[]): void;
    remove(...tokens: string[]): void;
    supports(token: string): boolean;
  }
  interface SkContentWindow {
    readonly document: unknown;
    postMessage(message: unknown, targetOrigin: string | { targetOrigin?: string }, transfer?: unknown[]): void;
  }
  interface SkIframeElement extends SkElement {
    get sandbox(): SkTokenList;
    set sandbox(value: string);
    allow: string;
    referrerPolicy: string;
    src: string;
    readonly contentWindow: SkContentWindow | null;
    readonly contentDocument: unknown;
    onload: ((ev: SkEvent) => unknown) | null;
    onerror: ((ev: SkEvent) => unknown) | null;
  }

  var HTMLIFrameElement: { new (): SkIframeElement; prototype: SkIframeElement };

  interface SkMediaElement {
    src: string;
  }

  interface SkFontFace {
    readonly family: string;
    readonly status: string;
    readonly unicodeRange: string;
    readonly loaded: Promise<SkFontFace>;
    load(): Promise<SkFontFace>;
  }
  interface SkFontFaceSetEvent extends SkEvent {
    readonly fontfaces: Iterable<SkFontFace>;
  }
  interface SkFontFaceSet extends SkEventTarget, Iterable<SkFontFace> {
    readonly status: string;
    readonly size: number;
    readonly ready: Promise<SkFontFaceSet>;
    add(face: SkFontFace): void;
    has(face: SkFontFace): boolean;
    check(font: string, text?: string): boolean;
    load(font: string, text?: string): Promise<SkFontFace[]>;
  }
  var FontFace: { new (family: string, source: string | ArrayBuffer): SkFontFace; prototype: SkFontFace };

  interface SkDomDocument extends SkEventTarget {
    readonly documentElement: SkElement;
    readonly body: SkElement;
    readonly activeElement: SkElement | null;
    readonly fonts: SkFontFaceSet;
    getElementById(id: string): SkElement | null;
    getElementsByTagName(tag: string): SkElementList;
    getElementsByClassName(names: string): SkElementList;
    getElementsByName(name: string): SkNodeList;
    createElement(tag: "iframe"): SkIframeElement;
    createElement(tag: string): SkElement;
    createElementNS(ns: string, tag: string): SkElement;
    querySelector<E extends SkElement = SkElement>(selector: string): E | null;
    querySelectorAll<E extends SkElement = SkElement>(selector: string): SkElementList<E>;
  }

  interface SkResizeObserverEntry {
    readonly target: SkElement;
    readonly contentRect: SkDomRect;
  }
  interface SkResizeObserver {
    observe(el: SkElement): void;
    disconnect(): void;
  }
  type ResizeObserver = SkResizeObserver;
  var ResizeObserver: { new (callback: (entries: SkResizeObserverEntry[]) => void): SkResizeObserver };
  interface SkIntersectionObserverEntry {
    readonly target: SkElement;
    readonly isIntersecting: boolean;
    readonly boundingClientRect: SkDomRect;
  }
  var IntersectionObserver: {
    new (callback: (entries: SkIntersectionObserverEntry[]) => void): {
      observe(el: SkElement): void;
      disconnect(): void;
    };
  };

  interface SkClipboardItem {
    readonly types: readonly string[];
    getType(type: string): Promise<SkBlob>;
  }
  var ClipboardItem: { new (items: Record<string, SkBlob>): SkClipboardItem; prototype: SkClipboardItem };

  interface SkMediaStreamConstraints {
    audio?: boolean;
    video?: boolean;
  }
  interface SkMediaTrackSettings {
    readonly echoCancellation?: boolean;
    readonly noiseSuppression?: boolean;
    readonly autoGainControl?: boolean;
    readonly channelCount?: number;
  }
  interface SkMediaStreamTrack {
    readonly readyState: string;
    readonly label: string;
    getSettings(): SkMediaTrackSettings;
    stop(): void;
  }
  interface SkMediaStream {
    getAudioTracks(): SkMediaStreamTrack[];
    getTracks(): SkMediaStreamTrack[];
  }
  interface SkMediaDeviceInfo {
    readonly kind: string;
    readonly deviceId: string;
    readonly label: string;
  }

  interface SkNavigator {
    readonly mediaDevices: {
      getUserMedia(constraints?: SkMediaStreamConstraints): Promise<SkMediaStream>;
      enumerateDevices(): Promise<SkMediaDeviceInfo[]>;
    };
  }
}
