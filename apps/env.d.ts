// `packages/vue-native/src/engine.d.ts` is deliberately not this file. NOTHING IS DECLARED THAT
// THE HOST DOES NOT PROVIDE.

export {};

// the window, as a global EventTarget — crates/vn-app/src/window.prelude.js The prelude installs
// these on `globalThis` before the app bundle evaluates.

declare global {
  function getSelection(): { toString(): string } | null;
  function addEventListener(
    type: string,
    listener: (ev: VnEvent) => void,
    options?: boolean | { capture?: boolean; once?: boolean },
  ): void;
  function removeEventListener(
    type: string,
    listener: (ev: VnEvent) => void,
    options?: boolean | { capture?: boolean },
  ): void;
  function dispatchEvent(ev: VnEvent): boolean;

  var innerWidth: number;
  var innerHeight: number;
  var outerWidth: number;
  var outerHeight: number;
  var screenX: number;
  var screenY: number;
  var devicePixelRatio: number;

  function resizeBy(dw: number, dh: number): void;

  function open(url?: string, target?: string, features?: string): null;

  interface VnLaunchParams {
    readonly targetURL: string | null;
    readonly files: readonly (VnFileSystemFileHandle | VnFileSystemDirectoryHandle)[];
  }
  var launchQueue: {
    setConsumer(consumer: (params: VnLaunchParams) => unknown): void;
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
  function matchMedia(query: string): VnMediaQueryList;

  interface VnMediaQueryList extends VnEventTarget {
    readonly matches: boolean;
    readonly media: string;
  }

  var performance: { now(): number };

  var ononline: ((ev: VnEvent) => unknown) | null;
  var onoffline: ((ev: VnEvent) => unknown) | null;
}

declare global {
  interface VnNavigator {
    windowControlsOverlay: VnWindowControlsOverlay;

    /**
     * The async Clipboard API — vendor/redjs/redjs/src/webapi/clipboard.rs, over the host's own
     * clipboard (the same one the native `<input>` uses for Ctrl+C: one owner, not two).
     */
    clipboard: {
      readText(): Promise<string>;
      writeText(text: string): Promise<void>;
      read(): Promise<unknown[]>;
      write(items: unknown[]): Promise<void>;
    };

    /** `el.click()` and `dispatchEvent` never activate. */
    readonly userActivation: {
      readonly isActive: boolean;
      readonly hasBeenActive: boolean;
    };

    readonly onLine: boolean;
  }

  interface VnWindowControlsOverlay extends VnEventTarget {
    readonly visible: boolean;
    getTitlebarAreaRect(): VnWindowRect;
  }

  interface VnWindowChrome {
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
  interface VnDocument {
    readonly fullscreenElement: VnElement | null;
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
  interface VnDomRect {
    readonly x: number;
    readonly y: number;
    readonly top: number;
    readonly left: number;
    readonly right: number;
    readonly bottom: number;
    readonly width: number;
    readonly height: number;
  }

  interface VnScrollToOptions {
    left?: number;
    top?: number;
    behavior?: "auto" | "instant" | "smooth";
  }

  interface VnScrollIntoViewOptions {
    block?: "start" | "center" | "end" | "nearest";
    inline?: "start" | "center" | "end" | "nearest";
    behavior?: "auto" | "instant" | "smooth";
  }

  interface VnElement extends VnEventTarget {
    readonly id: VnNodeId;
    getBoundingClientRect(): VnDomRect;
    /**
     * `0` on an element that does not scroll, and a write to one does nothing, exactly as in a
     * browser.
     */
    scrollTop: number;
    scrollLeft: number;
    readonly scrollWidth: number;
    readonly scrollHeight: number;
    scrollTo(x?: number | VnScrollToOptions, y?: number): void;
    scrollBy(x?: number | VnScrollToOptions, y?: number): void;
    scrollIntoView(arg?: boolean | VnScrollIntoViewOptions): void;
    focus(): void;
    blur(): void;
    click(): void;
    requestFullscreen(): Promise<void>;
  }

  interface VnMediaElement extends VnElement {
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
    readonly error: VnMediaError | null;
    play(): Promise<void>;
    pause(): void;
    canPlayType(type: string): string;
  }

  interface VnMediaError {
    readonly code: number;
    readonly message: string;
  }

  interface VnMediaEvent extends VnEvent {
    readonly currentTime: number;
    readonly duration: number;
    readonly readyState: number;
    readonly playbackRate: number;
  }
  interface VnLottieLoopEvent extends VnEvent {
    readonly currentTime: number;
    readonly totalTime: number;
    readonly currentLoop: number;
  }
}

declare global {
  interface VnStorage {
    readonly length: number;
    key(index: number): string | null;
    getItem(key: string): string | null;
    /** Typed strictly so the coercion is written down at the call site rather than being invisible. */
    setItem(key: string, value: string): void;
    removeItem(key: string): void;
    clear(): void;
  }
  var localStorage: VnStorage;
  /** Per-run, never written anywhere. */
  var sessionStorage: VnStorage;
}

declare global {
  interface VnFetchResponse {
    arrayBuffer(): Promise<ArrayBuffer>;
  }

  /**
   * So unlike `fetch` and `AudioContext` next door this one is not `| undefined`: a redjs that
   * lacks it does not exist, and apps/devtools' request timeout depends on it.
   */
  interface VnAbortSignal extends VnEventTarget {
    readonly aborted: boolean;
    readonly reason: unknown;
    throwIfAborted(): void;
  }
  interface VnAbortController {
    readonly signal: VnAbortSignal;
    abort(reason?: unknown): void;
  }
  var AbortController: { new (): VnAbortController; prototype: VnAbortController };
}

declare global {
  type VnBlobPart = VnBlob | ArrayBuffer | ArrayBufferView | string;
  interface VnBlobPropertyBag {
    type?: string;
    endings?: "transparent" | "native";
  }
  interface VnByteStream extends AsyncIterable<Uint8Array> {
    getReader(): {
      read(): Promise<{ done: boolean; value?: Uint8Array }>;
      cancel(reason?: unknown): Promise<void>;
      releaseLock(): void;
    };
    cancel(reason?: unknown): Promise<void>;
    readonly locked: boolean;
  }
  interface VnBlob {
    readonly size: number;
    readonly type: string;
    arrayBuffer(): Promise<ArrayBuffer>;
    bytes(): Promise<Uint8Array>;
    text(): Promise<string>;
    slice(start?: number, end?: number, contentType?: string): VnBlob;
    stream(): VnByteStream;
  }
  var Blob: { new (parts?: Iterable<VnBlobPart>, options?: VnBlobPropertyBag): VnBlob; prototype: VnBlob };

  interface VnFile extends VnBlob {
    readonly name: string;
    readonly lastModified: number;
    readonly lastModifiedDate: Date;
    readonly webkitRelativePath: string;
  }
  var File: {
    new (bits: Iterable<VnBlobPart>, name: string, options?: VnBlobPropertyBag & { lastModified?: number }): VnFile;
    prototype: VnFile;
  };

  interface VnFileList extends Iterable<VnFile> {
    readonly length: number;
    item(index: number): VnFile | null;
    readonly [index: number]: VnFile;
  }
  var FileList: { new (): VnFileList; prototype: VnFileList };

  interface VnURLConstructor {
    createObjectURL(blob: VnBlob): string;
    revokeObjectURL(url: string): void;
  }

  interface VnDataTransferItem {
    readonly kind: "string" | "file" | "";
    readonly type: string;
    getAsString(callback: ((data: string) => void) | null): void;
    getAsFile(): VnFile | null;
    getAsFileSystemHandle?(): Promise<unknown | null>;
  }
  interface VnDataTransferItemList extends Iterable<VnDataTransferItem> {
    readonly length: number;
    readonly [index: number]: VnDataTransferItem;
    add(data: string, type: string): VnDataTransferItem | null;
    add(file: VnFile): VnDataTransferItem | null;
    remove(index: number): void;
    clear(): void;
  }
  interface VnDataTransfer {
    dropEffect: "none" | "copy" | "link" | "move";
    effectAllowed: "none" | "copy" | "copyLink" | "copyMove" | "link" | "linkMove" | "move" | "all" | "uninitialized";
    readonly items: VnDataTransferItemList;
    readonly types: readonly string[];
    readonly files: VnFileList;
    getData(format: string): string;
    setData(format: string, data: string): void;
    clearData(format?: string): void;
    setDragImage(image: unknown, x: number, y: number): void;
  }
  var DataTransfer: { new (): VnDataTransfer; prototype: VnDataTransfer };

  interface VnFileInputElement extends VnElement {
    files: VnFileList;
    value: string;
    accept: string;
    multiple: boolean;
    webkitdirectory: boolean;
    /** Needs a user activation (NotAllowedError otherwise). */
    showPicker(): void;
  }

  interface VnCloseEvent extends VnEvent {
    readonly code: number;
    readonly reason: string;
    readonly wasClean: boolean;
  }
  interface VnMessageEvent extends VnEvent {
    readonly data: string | VnBlob | ArrayBuffer;
  }

  interface VnWebSocket extends VnEventTarget {
    readonly readyState: number;
    readonly bufferedAmount: number;
    readonly protocol: string;
    readonly url: string;
    binaryType: "blob" | "arraybuffer";
    onopen: ((ev: VnEvent) => void) | null;
    onerror: ((ev: VnEvent) => void) | null;
    onclose: ((ev: VnCloseEvent) => void) | null;
    onmessage: ((ev: VnMessageEvent) => void) | null;
    send(data: string | ArrayBuffer | ArrayBufferView | VnBlob): void;
    close(code?: number, reason?: string): void;
  }
  var WebSocket: {
    new (url: string, protocols?: string | string[]): VnWebSocket;
    prototype: VnWebSocket;
    readonly CONNECTING: 0;
    readonly OPEN: 1;
    readonly CLOSING: 2;
    readonly CLOSED: 3;
  };
}

// Every picker needs a user activation, else SecurityError.

declare global {
  type VnPermissionState = "granted" | "prompt";
  interface VnFileSystemHandlePermissionDescriptor {
    mode?: "read" | "readwrite";
  }
  interface VnFileSystemHandle {
    readonly kind: "file" | "directory";
    readonly name: string;
    isSameEntry(other: VnFileSystemHandle): Promise<boolean>;
    queryPermission(descriptor?: VnFileSystemHandlePermissionDescriptor): Promise<VnPermissionState>;
    requestPermission(descriptor?: VnFileSystemHandlePermissionDescriptor): Promise<VnPermissionState>;
  }
  interface VnFileSystemFileHandle extends VnFileSystemHandle {
    readonly kind: "file";
    getFile(): Promise<VnFile>;
    createWritable(options?: { keepExistingData?: boolean }): Promise<VnFileSystemWritableFileStream>;
  }
  interface VnFileSystemDirectoryHandle
    extends VnFileSystemHandle,
      AsyncIterable<[string, VnFileSystemFileHandle | VnFileSystemDirectoryHandle]> {
    readonly kind: "directory";
    getFileHandle(name: string, options?: { create?: boolean }): Promise<VnFileSystemFileHandle>;
    getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<VnFileSystemDirectoryHandle>;
    removeEntry(name: string, options?: { recursive?: boolean }): Promise<void>;
    resolve(possibleDescendant: VnFileSystemHandle): Promise<string[] | null>;
    entries(): AsyncIterableIterator<[string, VnFileSystemFileHandle | VnFileSystemDirectoryHandle]>;
    keys(): AsyncIterableIterator<string>;
    values(): AsyncIterableIterator<VnFileSystemFileHandle | VnFileSystemDirectoryHandle>;
  }
  type VnFileSystemWriteData = VnBlob | ArrayBuffer | ArrayBufferView | string;
  type VnFileSystemWriteChunk =
    | VnFileSystemWriteData
    | { type: "write"; position?: number | null; data: VnFileSystemWriteData }
    | { type: "seek"; position: number }
    | { type: "truncate"; size: number };
  interface VnFileSystemWritableFileStream {
    readonly locked: boolean;
    write(data: VnFileSystemWriteChunk): Promise<void>;
    seek(position: number): Promise<void>;
    truncate(size: number): Promise<void>;
    close(): Promise<void>;
    abort(reason?: unknown): Promise<void>;
  }
  // The construct signatures are lib.dom's: calling one throws "Illegal constructor", but without
  // it `x instanceof FileSystemFileHandle` does not type-check.
  var FileSystemHandle: { new (): VnFileSystemHandle; prototype: VnFileSystemHandle };
  var FileSystemFileHandle: { new (): VnFileSystemFileHandle; prototype: VnFileSystemFileHandle };
  var FileSystemDirectoryHandle: { new (): VnFileSystemDirectoryHandle; prototype: VnFileSystemDirectoryHandle };
  var FileSystemWritableFileStream: { new (): VnFileSystemWritableFileStream; prototype: VnFileSystemWritableFileStream };

  type VnWellKnownDirectory = "desktop" | "documents" | "downloads" | "music" | "pictures" | "videos";
  interface VnFilePickerAcceptType {
    description?: string;
    accept: Record<string, string | string[]>;
  }
  interface VnFilePickerOptions {
    id?: string;
    startIn?: VnWellKnownDirectory | VnFileSystemHandle;
    types?: VnFilePickerAcceptType[];
    excludeAcceptAllOption?: boolean;
  }
  interface VnOpenFilePickerOptions extends VnFilePickerOptions {
    multiple?: boolean;
  }
  interface VnSaveFilePickerOptions extends VnFilePickerOptions {
    suggestedName?: string;
  }
  interface VnDirectoryPickerOptions {
    id?: string;
    startIn?: VnWellKnownDirectory | VnFileSystemHandle;
    mode?: "read" | "readwrite";
  }
  function showOpenFilePicker(options?: VnOpenFilePickerOptions): Promise<VnFileSystemFileHandle[]>;
  function showSaveFilePicker(options?: VnSaveFilePickerOptions): Promise<VnFileSystemFileHandle>;
  function showDirectoryPicker(options?: VnDirectoryPickerOptions): Promise<VnFileSystemDirectoryHandle>;

  interface VnProgressEvent extends VnEvent {
    readonly lengthComputable: boolean;
    readonly loaded: number;
    readonly total: number;
  }
  var ProgressEvent: {
    new (type: string, init?: VnEventInit & { lengthComputable?: boolean; loaded?: number; total?: number }): VnProgressEvent;
    prototype: VnProgressEvent;
  };

  interface VnFileReader extends VnEventTarget {
    readonly readyState: 0 | 1 | 2;
    readonly result: string | ArrayBuffer | null;
    readonly error: VnDOMException | null;
    readonly EMPTY: 0;
    readonly LOADING: 1;
    readonly DONE: 2;
    readAsArrayBuffer(blob: VnBlob): void;
    readAsBinaryString(blob: VnBlob): void;
    /** `encoding`, else the Blob type's charset, else UTF-8; a BOM wins over all three. */
    readAsText(blob: VnBlob, encoding?: string): void;
    readAsDataURL(blob: VnBlob): void;
    abort(): void;
    onloadstart: ((ev: VnProgressEvent) => void) | null;
    onprogress: ((ev: VnProgressEvent) => void) | null;
    onload: ((ev: VnProgressEvent) => void) | null;
    onerror: ((ev: VnProgressEvent) => void) | null;
    onabort: ((ev: VnProgressEvent) => void) | null;
    onloadend: ((ev: VnProgressEvent) => void) | null;
  }
  var FileReader: {
    new (): VnFileReader;
    prototype: VnFileReader;
    readonly EMPTY: 0;
    readonly LOADING: 1;
    readonly DONE: 2;
  };
}

declare global {
  interface VnAudioContext {
    readonly state: "suspended" | "running" | "closed";
    readonly sampleRate: number;
    readonly currentTime: number;
    readonly audioWorklet?: unknown;
    createOscillator(): VnOscillatorNode;
    createAnalyser(): VnAnalyserNode;
    createBuffer(channels: number, frames: number, sampleRate: number): VnAudioBuffer;
    createBufferSource(): VnAudioBufferSourceNode;
    createChannelSplitter(numberOfOutputs?: number): VnChannelSplitterNode;
    createChannelMerger(numberOfInputs?: number): VnChannelMergerNode;
    decodeAudioData(bytes: ArrayBuffer): Promise<VnAudioBuffer>;
    resume(): Promise<void>;
    suspend(): Promise<void>;
    createRedStreamSource(options?: Partial<VnRedStreamSourceOptions>): VnRedStreamSourceNode;
  }

  interface VnRedStreamSourceNode {
    push(block: Float32Array): number;
    readonly queuedSeconds?: number;
    readonly ready: boolean;
    readonly underflowed: boolean;
    readonly overflowed: boolean;
  }

  /**
   * NOT a merge onto engine.d.ts's `VnGainNode`, which already declares `readonly gain: { value:
   * number }` — the `.value` half — and a merge cannot narrow a property that is already there.
   */
  interface VnGainNodeWithParam extends VnAudioNode {
    readonly gain: VnAudioParam;
  }

  interface VnAudioParam {
    value: number;
    cancelScheduledValues(when: number): void;
    setValueAtTime(value: number, when: number): void;
    linearRampToValueAtTime(value: number, when: number): void;
    exponentialRampToValueAtTime(value: number, when: number): void;
  }

  interface VnOscillatorNode extends VnAudioNode {
    type: string;
    readonly frequency: VnAudioParam;
    readonly detune: VnAudioParam;
    onended: (() => void) | null;
    start(when?: number): void;
    stop(when?: number): void;
  }

  interface VnAnalyserNode extends VnAudioNode {
    fftSize: number;
    smoothingTimeConstant: number;
    readonly frequencyBinCount: number;
    getByteFrequencyData(into: Uint8Array): void;
    getFloatTimeDomainData(into: Float32Array): void;
  }

  interface VnAudioBuffer {
    readonly numberOfChannels: number;
    readonly length: number;
    readonly sampleRate: number;
    readonly duration: number;
    getChannelData(channel: number): Float32Array;
    copyToChannel(source: Float32Array, channel: number): void;
  }

  interface VnAudioBufferSourceNode extends VnAudioNode {
    buffer: VnAudioBuffer | null;
    onended: (() => void) | null;
    start(when?: number): void;
    stop(when?: number): void;
  }

  interface VnAudioNode {
    readonly numberOfInputs: number;
    readonly numberOfOutputs: number;
    connect(to: VnAudioNode, output?: number, input?: number): unknown;
    disconnect(outputOrDestination?: number | VnAudioNode, output?: number, input?: number): void;
  }

  interface VnChannelSplitterNode extends VnAudioNode {
    channelCount: number;
    channelCountMode: string;
    channelInterpretation: string;
  }

  interface VnChannelMergerNode extends VnAudioNode {
    channelCount: number;
    channelCountMode: string;
    channelInterpretation: string;
  }

  interface VnChannelNodeOptions {
    channelCount?: number;
    channelCountMode?: string;
    channelInterpretation?: string;
  }

  var ChannelSplitterNode: {
    new (context: VnAudioContext, options?: VnChannelNodeOptions & { numberOfOutputs?: number }): VnChannelSplitterNode;
    prototype: VnChannelSplitterNode;
  };
  var ChannelMergerNode: {
    new (context: VnAudioContext, options?: VnChannelNodeOptions & { numberOfInputs?: number }): VnChannelMergerNode;
    prototype: VnChannelMergerNode;
  };

  var AudioContext: { new (): VnAudioContext; prototype: VnAudioContext } | undefined;
  /** Declared so the probe compiles; `undefined` is the honest type. */
  var OfflineAudioContext: undefined;
}

declare global {
  /**
   * It is for an app's own adaptations to the host, `if (globalThis.isVn) …`, without importing
   * "vue-native" (which a browser build of the same app cannot resolve).
   */
  var isVn: true;

  var __VN_SCENARIO__: string | undefined;

  /**
   * `VN_HTTP_ORIGIN` — where MediaPage points its http(s) `src` demos, so the page can be aimed at
   * a real server without an internet dependency.
   */
  var __VN_HTTP_ORIGIN__: string | undefined;

  var __vn_perf_mark: ((name: string) => void) | undefined;

  /**
   * The scenario driver reaches VoicePage's verbs through `ctl.ts eval`, so the page publishes
   * them under a known name.
   */
  var __voice: Record<string, (...args: never[]) => unknown> | undefined;
}
