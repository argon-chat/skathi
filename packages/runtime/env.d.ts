// The web surface a Skathi app sees, under the web's names, on top of src/engine.d.ts. NOTHING IS
// DECLARED THAT THE HOST DOES NOT PROVIDE: lib.dom stays out because it declares far more.
// WebGPU is the exception: @webgpu/types declares a few calls the host lacks (the web-platform
// reference's WebGPU limitations), and VideoFrame below exists only so those types resolve.

/// <reference types="@webgpu/types" />

export {};

declare const noVideoFrame: unique symbol;

// The window: the prelude installs these on `globalThis` before the app bundle evaluates.

declare global {
  interface GlobalEventHandlersEventMap {
    abort: Event;
    animationcancel: AnimationEvent;
    animationend: AnimationEvent;
    animationiteration: AnimationEvent;
    animationstart: AnimationEvent;
    auxclick: MouseEvent;
    beforeinput: InputEvent;
    blur: FocusEvent;
    change: Event;
    click: MouseEvent;
    compositionend: CompositionEvent;
    compositionstart: CompositionEvent;
    compositionupdate: CompositionEvent;
    contextmenu: MouseEvent;
    dblclick: MouseEvent;
    drag: DragEvent;
    dragend: DragEvent;
    dragenter: DragEvent;
    dragleave: DragEvent;
    dragover: DragEvent;
    dragstart: DragEvent;
    drop: DragEvent;
    error: ErrorEvent;
    focus: FocusEvent;
    gesturestart: GestureEvent;
    gesturechange: GestureEvent;
    gestureend: GestureEvent;
    focusin: FocusEvent;
    focusout: FocusEvent;
    gotpointercapture: PointerEvent;
    input: Event;
    keydown: KeyboardEvent;
    keypress: KeyboardEvent;
    keyup: KeyboardEvent;
    load: Event;
    lostpointercapture: PointerEvent;
    mousedown: MouseEvent;
    mouseenter: MouseEvent;
    mouseleave: MouseEvent;
    mousemove: MouseEvent;
    mouseout: MouseEvent;
    mouseover: MouseEvent;
    mouseup: MouseEvent;
    pointercancel: PointerEvent;
    pointerdown: PointerEvent;
    pointerenter: PointerEvent;
    pointerleave: PointerEvent;
    pointermove: PointerEvent;
    pointerout: PointerEvent;
    pointerover: PointerEvent;
    pointerup: PointerEvent;
    reset: Event;
    resize: Event;
    scroll: Event;
    scrollend: Event;
    select: Event;
    /** Skathi: before `<input type=color>` shows its picker; `preventDefault()` keeps it closed. */
    showpicker: Event;
    submit: Event;
    surfacedisplaychange: SkSurfaceDisplayEvent;
    surfacelost: SkSurfaceEvent;
    surfaceready: SkSurfaceEvent;
    surfaceresize: SkSurfaceEvent;
    transitioncancel: TransitionEvent;
    transitionend: TransitionEvent;
    transitionrun: TransitionEvent;
    transitionstart: TransitionEvent;
    wheel: WheelEvent;
  }
  interface WindowEventMap extends GlobalEventHandlersEventMap {
    beforeunload: Event;
    offline: Event;
    online: Event;
    pagehide: Event;
    unload: Event;
  }
  interface DocumentEventMap extends GlobalEventHandlersEventMap {
    fullscreenchange: Event;
    fullscreenerror: Event;
    pointerlockchange: Event;
    pointerlockerror: Event;
    selectionchange: Event;
    visibilitychange: Event;
  }

  function getSelection(): { toString(): string } | null;
  function addEventListener<K extends keyof WindowEventMap>(
    type: K,
    listener: (ev: WindowEventMap[K]) => unknown,
    options?: boolean | AddEventListenerOptions,
  ): void;
  function addEventListener(
    type: string,
    listener: ((ev: Event) => unknown) | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  function removeEventListener<K extends keyof WindowEventMap>(
    type: K,
    listener: (ev: WindowEventMap[K]) => unknown,
    options?: boolean | EventListenerOptions,
  ): void;
  function removeEventListener(
    type: string,
    listener: ((ev: Event) => unknown) | null,
    options?: boolean | EventListenerOptions,
  ): void;
  function dispatchEvent(event: Event): boolean;

  var innerWidth: number;
  var innerHeight: number;
  var outerWidth: number;
  var outerHeight: number;
  var screenX: number;
  var screenY: number;
  var screenLeft: number;
  var screenTop: number;
  var devicePixelRatio: number;

  /** The outer size, CSS px, as on the web. */
  function resizeTo(width: number, height: number): void;
  function resizeBy(x: number, y: number): void;
  function moveTo(x: number, y: number): void;
  function moveBy(x: number, y: number): void;
  /** Closes the app: `pagehide`, then the window goes. Not asked through `close-requested`. */
  function close(): void;
  /** Raises only under `window.focus: "raise"`; otherwise requests attention. */
  function focus(): void;
  /** A no-op, as in every browser. */
  function blur(): void;
  /** Not implemented: rejects with `NotSupportedError`. */
  function getScreenDetails(): Promise<never>;

  function open(url?: string, target?: string, features?: string): null;

  interface LaunchParams {
    readonly targetURL: string | null;
    readonly files: readonly FileSystemHandle[];
  }
  interface LaunchQueue {
    setConsumer(consumer: (params: LaunchParams) => unknown): void;
  }
  var LaunchParams: { new (): LaunchParams; prototype: LaunchParams };
  var LaunchQueue: { new (): LaunchQueue; prototype: LaunchQueue };
  var launchQueue: LaunchQueue;

  interface ScreenOrientation extends EventTarget {
    readonly type: "landscape-primary" | "portrait-primary";
    readonly angle: number;
    lock(orientation: string): Promise<void>;
    unlock(): void;
  }
  interface Screen extends EventTarget {
    readonly width: number;
    readonly height: number;
    readonly availWidth: number;
    readonly availHeight: number;
    readonly availLeft: number;
    readonly availTop: number;
    readonly colorDepth: number;
    readonly pixelDepth: number;
    readonly isExtended: boolean;
    readonly orientation: ScreenOrientation;
  }
  var Screen: { new (): Screen; prototype: Screen };
  var screen: Screen;

  /** A query the evaluator cannot run advises rather than answering false forever. */
  function matchMedia(query: string): MediaQueryList;

  interface MediaQueryListEvent extends Event {
    readonly matches: boolean;
    readonly media: string;
  }
  interface MediaQueryList extends EventTarget {
    readonly matches: boolean;
    readonly media: string;
    onchange: ((ev: MediaQueryListEvent) => unknown) | null;
    addListener(callback: ((ev: MediaQueryListEvent) => unknown) | null): void;
    removeListener(callback: ((ev: MediaQueryListEvent) => unknown) | null): void;
  }
  var MediaQueryList: { new (): MediaQueryList; prototype: MediaQueryList };

  type PressureSource = "cpu";
  type PressureState = "nominal" | "fair" | "serious" | "critical";

  interface PressureObserverOptions {
    sampleInterval?: number;
  }

  interface PressureRecord {
    readonly source: PressureSource;
    readonly state: PressureState;
    readonly time: number;
    toJSON(): { source: PressureSource; state: PressureState; time: number };
  }
  var PressureRecord: { prototype: PressureRecord };

  interface PressureObserver {
    observe(source: PressureSource, options?: PressureObserverOptions): Promise<void>;
    unobserve(source: PressureSource): void;
    disconnect(): void;
    takeRecords(): PressureRecord[];
  }
  var PressureObserver: {
    new (callback: (records: PressureRecord[], observer: PressureObserver) => void): PressureObserver;
    readonly knownSources: readonly PressureSource[];
    prototype: PressureObserver;
  };

  var performance: { now(): number };

  var ononline: ((ev: Event) => unknown) | null;
  var onoffline: ((ev: Event) => unknown) | null;
}

declare global {
  interface Clipboard extends EventTarget {
    readText(): Promise<string>;
    writeText(data: string): Promise<void>;
    read(): Promise<ClipboardItem[]>;
    write(data: ClipboardItem[]): Promise<void>;
  }

  interface UserActivation {
    readonly isActive: boolean;
    readonly hasBeenActive: boolean;
  }

  interface Navigator {
    readonly windowControlsOverlay: WindowControlsOverlay;
    /** Over the host's own clipboard, the same one a native `<input>` uses for Ctrl+C. */
    readonly clipboard: Clipboard;
    /** `el.click()` and `dispatchEvent` never activate. */
    readonly userActivation: UserActivation;
    readonly onLine: boolean;
    readonly platform: string;
    readonly maxTouchPoints: number;
    readonly mediaDevices: MediaDevices;
    /** The taskbar button's badge on Windows, the Dock icon's on macOS. No argument shows a dot. */
    setAppBadge(contents?: number): Promise<void>;
    clearAppBadge(): Promise<void>;
  }

  interface WindowControlsOverlay extends EventTarget {
    readonly visible: boolean;
    getTitlebarAreaRect(): DOMRect;
    ongeometrychange: ((ev: Event) => unknown) | null;
  }

  /** `appWindow.chrome`: the window's titlebar and inset state. Not web API. */
  interface SkWindowChrome {
    titleBarStyle: string;
    displayMode: string;
    /** The Window Controls Overlay area's height (`env(titlebar-area-height)`): 0 while the area is not visible (OS chrome, fullscreen, minimized, or the fallback title bar still up). */
    titleBarHeight: number;
    fallbackTitleBar: boolean;
    overlayVisible: boolean;
    /** The host draws the caption buttons (the `titleBarOverlay` cluster, the macOS traffic lights): draw none of your own. */
    hostControls: boolean;
    client: { width: number; height: number };
    contentInset: { left: number; top: number; right: number; bottom: number };
  }
}

declare global {
  interface DOMRectReadOnly {
    readonly x: number;
    readonly y: number;
    readonly top: number;
    readonly left: number;
    readonly right: number;
    readonly bottom: number;
    readonly width: number;
    readonly height: number;
    toJSON(): object;
  }
  interface DOMRect extends DOMRectReadOnly {
    x: number;
    y: number;
    width: number;
    height: number;
  }
  var DOMRectReadOnly: {
    new (x?: number, y?: number, width?: number, height?: number): DOMRectReadOnly;
    prototype: DOMRectReadOnly;
  };
  var DOMRect: { new (x?: number, y?: number, width?: number, height?: number): DOMRect; prototype: DOMRect };

  interface DOMRectList extends Iterable<DOMRect> {
    readonly length: number;
    item(index: number): DOMRect | null;
    readonly [index: number]: DOMRect;
  }
  var DOMRectList: { new (): DOMRectList; prototype: DOMRectList };

  type ScrollBehavior = "auto" | "instant" | "smooth";
  type ScrollLogicalPosition = "start" | "center" | "end" | "nearest";
  interface ScrollOptions {
    behavior?: ScrollBehavior;
  }
  interface ScrollToOptions extends ScrollOptions {
    left?: number;
    top?: number;
  }
  interface ScrollIntoViewOptions extends ScrollOptions {
    block?: ScrollLogicalPosition;
    inline?: ScrollLogicalPosition;
  }
  interface FocusOptions {
    preventScroll?: boolean;
  }
  interface FullscreenOptions {
    navigationUI?: "auto" | "hide" | "show";
  }
  interface PointerLockOptions {
    unadjustedMovement?: boolean;
  }

  interface NodeList extends Iterable<Node> {
    readonly length: number;
    item(index: number): Node | null;
    readonly [index: number]: Node;
    forEach(callbackfn: (value: Node, key: number, parent: NodeList) => void): void;
  }
  interface NodeListOf<TNode extends Node> extends NodeList, Iterable<TNode> {
    item(index: number): TNode;
    readonly [index: number]: TNode;
    forEach(callbackfn: (value: TNode, key: number, parent: NodeListOf<TNode>) => void): void;
    [Symbol.iterator](): IterableIterator<TNode>;
  }
  var NodeList: { new (): NodeList; prototype: NodeList };

  interface HTMLCollection extends Iterable<Element> {
    readonly length: number;
    item(index: number): Element | null;
    namedItem(name: string): Element | null;
    readonly [index: number]: Element;
  }
  interface HTMLCollectionOf<T extends Element> extends HTMLCollection, Iterable<T> {
    item(index: number): T | null;
    namedItem(name: string): T | null;
    readonly [index: number]: T;
    [Symbol.iterator](): IterableIterator<T>;
  }
  var HTMLCollection: { new (): HTMLCollection; prototype: HTMLCollection };

  interface DOMTokenList extends Iterable<string> {
    value: string;
    readonly length: number;
    item(index: number): string | null;
    contains(token: string): boolean;
    add(...tokens: string[]): void;
    remove(...tokens: string[]): void;
    toggle(token: string, force?: boolean): boolean;
    replace(token: string, newToken: string): boolean;
    supports(token: string): boolean;
  }
  var DOMTokenList: { new (): DOMTokenList; prototype: DOMTokenList };

  interface DOMStringMap {
    [name: string]: string | undefined;
  }

  /** Any property assignment reaches the host's style; the common ones are typed. */
  interface CSSStyleDeclaration {
    cssText: string;
    readonly length: number;
    item(index: number): string;
    getPropertyValue(property: string): string;
    getPropertyPriority(property: string): string;
    setProperty(property: string, value: string | null, priority?: string): void;
    removeProperty(property: string): string;
    alignContent: string;
    alignItems: string;
    alignSelf: string;
    animation: string;
    aspectRatio: string;
    background: string;
    backgroundColor: string;
    backgroundImage: string;
    border: string;
    borderColor: string;
    borderRadius: string;
    borderWidth: string;
    bottom: string;
    boxShadow: string;
    boxSizing: string;
    clipPath: string;
    color: string;
    columnGap: string;
    cursor: string;
    display: string;
    filter: string;
    flex: string;
    flexBasis: string;
    flexDirection: string;
    flexGrow: string;
    flexShrink: string;
    flexWrap: string;
    fontFamily: string;
    fontSize: string;
    fontStyle: string;
    fontWeight: string;
    gap: string;
    gridTemplateColumns: string;
    gridTemplateRows: string;
    height: string;
    inset: string;
    justifyContent: string;
    left: string;
    letterSpacing: string;
    lineHeight: string;
    margin: string;
    marginBottom: string;
    marginLeft: string;
    marginRight: string;
    marginTop: string;
    maxHeight: string;
    maxWidth: string;
    minHeight: string;
    minWidth: string;
    objectFit: string;
    opacity: string;
    outline: string;
    overflow: string;
    overflowX: string;
    overflowY: string;
    padding: string;
    paddingBottom: string;
    paddingLeft: string;
    paddingRight: string;
    paddingTop: string;
    pointerEvents: string;
    position: string;
    right: string;
    rowGap: string;
    textAlign: string;
    textDecoration: string;
    textOverflow: string;
    top: string;
    transform: string;
    transformOrigin: string;
    transition: string;
    userSelect: string;
    visibility: string;
    whiteSpace: string;
    width: string;
    willChange: string;
    zIndex: string;
  }

  interface Keyframe {
    offset?: number | null;
    easing?: string;
    composite?: "replace" | "add" | "accumulate" | "auto";
    [property: string]: string | number | null | undefined;
  }
  interface PropertyIndexedKeyframes {
    offset?: number | (number | null)[];
    easing?: string | string[];
    composite?: string | string[];
    [property: string]: string | string[] | number | null | (number | null)[] | undefined;
  }
  interface KeyframeAnimationOptions {
    id?: string;
    duration?: number;
    easing?: string;
    delay?: number;
    endDelay?: number;
    iterations?: number;
    iterationStart?: number;
    direction?: "normal" | "reverse" | "alternate" | "alternate-reverse";
    fill?: "none" | "forwards" | "backwards" | "both" | "auto";
  }
  interface Animation extends EventTarget {
    id: string;
    readonly playState: "idle" | "running" | "paused" | "finished";
    readonly pending: boolean;
    readonly ready: Promise<Animation>;
    readonly finished: Promise<Animation>;
    currentTime: number | null;
    startTime: number | null;
    playbackRate: number;
    onfinish: ((ev: Event) => unknown) | null;
    oncancel: ((ev: Event) => unknown) | null;
    play(): void;
    pause(): void;
    cancel(): void;
    finish(): void;
    reverse(): void;
    updatePlaybackRate(playbackRate: number): void;
  }

  interface Node extends EventTarget {
    readonly nodeType: number;
    readonly nodeName: string;
    readonly isConnected: boolean;
    readonly ownerDocument: Document | null;
    readonly parentNode: ParentNode | null;
    readonly parentElement: HTMLElement | null;
    readonly firstChild: ChildNode | null;
    readonly lastChild: ChildNode | null;
    readonly nextSibling: ChildNode | null;
    readonly previousSibling: ChildNode | null;
    readonly childNodes: NodeListOf<ChildNode>;
    textContent: string | null;
    appendChild<T extends Node>(node: T): T;
    insertBefore<T extends Node>(node: T, child: Node | null): T;
    removeChild<T extends Node>(child: T): T;
    replaceChild<T extends Node>(node: Node, child: T): T;
    cloneNode(deep?: boolean): Node;
    contains(other: Node | null): boolean;
    hasChildNodes(): boolean;
    getRootNode(): Node;
    readonly ELEMENT_NODE: 1;
    readonly TEXT_NODE: 3;
    readonly COMMENT_NODE: 8;
    readonly DOCUMENT_NODE: 9;
  }
  var Node: {
    new (): Node;
    prototype: Node;
    readonly ELEMENT_NODE: 1;
    readonly TEXT_NODE: 3;
    readonly COMMENT_NODE: 8;
    readonly DOCUMENT_NODE: 9;
  };

  interface ChildNode extends Node {
    after(...nodes: (Node | string)[]): void;
    before(...nodes: (Node | string)[]): void;
    remove(): void;
    replaceWith(...nodes: (Node | string)[]): void;
  }
  interface ParentNode extends Node {
    readonly children: HTMLCollection;
    readonly childElementCount: number;
    readonly firstElementChild: Element | null;
    readonly lastElementChild: Element | null;
    append(...nodes: (Node | string)[]): void;
    prepend(...nodes: (Node | string)[]): void;
    replaceChildren(...nodes: (Node | string)[]): void;
    querySelector<K extends keyof HTMLElementTagNameMap>(selectors: K): HTMLElementTagNameMap[K] | null;
    querySelector<E extends Element = Element>(selectors: string): E | null;
    querySelectorAll<K extends keyof HTMLElementTagNameMap>(selectors: K): NodeListOf<HTMLElementTagNameMap[K]>;
    querySelectorAll<E extends Element = Element>(selectors: string): NodeListOf<E>;
  }

  interface CharacterData extends ChildNode {
    data: string;
  }
  interface Text extends CharacterData {}
  interface Comment extends CharacterData {}
  var Text: { new (data?: string): Text; prototype: Text };
  var Comment: { new (data?: string): Comment; prototype: Comment };

  interface Element extends ParentNode, ChildNode {
    /** The `id` ATTRIBUTE; the host's node id is not reachable from script. */
    id: string;
    className: string;
    readonly classList: DOMTokenList;
    readonly tagName: string;
    readonly localName: string;
    readonly namespaceURI: string | null;
    readonly ownerDocument: Document;
    readonly clientWidth: number;
    readonly clientHeight: number;
    readonly clientLeft: number;
    readonly clientTop: number;
    /** `0` on an element that does not scroll, and a write to one does nothing, as in a browser. */
    scrollTop: number;
    scrollLeft: number;
    readonly scrollWidth: number;
    readonly scrollHeight: number;
    getAttribute(qualifiedName: string): string | null;
    getAttributeNames(): string[];
    hasAttribute(qualifiedName: string): boolean;
    hasAttributes(): boolean;
    setAttribute(qualifiedName: string, value: string): void;
    removeAttribute(qualifiedName: string): void;
    toggleAttribute(qualifiedName: string, force?: boolean): boolean;
    matches(selectors: string): boolean;
    closest<K extends keyof HTMLElementTagNameMap>(selector: K): HTMLElementTagNameMap[K] | null;
    closest<E extends Element = Element>(selectors: string): E | null;
    getElementsByClassName(classNames: string): HTMLCollectionOf<Element>;
    getElementsByTagName<K extends keyof HTMLElementTagNameMap>(
      qualifiedName: K,
    ): HTMLCollectionOf<HTMLElementTagNameMap[K]>;
    getElementsByTagName(qualifiedName: string): HTMLCollectionOf<Element>;
    getBoundingClientRect(): DOMRect;
    getClientRects(): DOMRectList;
    scroll(options?: ScrollToOptions): void;
    scroll(x: number, y: number): void;
    scrollTo(options?: ScrollToOptions): void;
    scrollTo(x: number, y: number): void;
    scrollBy(options?: ScrollToOptions): void;
    scrollBy(x: number, y: number): void;
    scrollIntoView(arg?: boolean | ScrollIntoViewOptions): void;
    insertAdjacentElement(
      where: "beforebegin" | "afterbegin" | "beforeend" | "afterend",
      element: Element,
    ): Element | null;
    insertAdjacentText(where: "beforebegin" | "afterbegin" | "beforeend" | "afterend", data: string): void;
    setPointerCapture(pointerId: number): void;
    releasePointerCapture(pointerId: number): void;
    hasPointerCapture(pointerId: number): boolean;
    requestFullscreen(options?: FullscreenOptions): Promise<void>;
    requestPointerLock(options?: PointerLockOptions): Promise<void>;
    animate(
      keyframes: Keyframe[] | PropertyIndexedKeyframes | null,
      options?: number | KeyframeAnimationOptions,
    ): Animation;
    getAnimations(options?: { subtree?: boolean }): Animation[];
    cloneNode(deep?: boolean): Element;
  }
  var Element: { new (): Element; prototype: Element };

  interface HTMLElement extends Element {
    readonly style: CSSStyleDeclaration;
    readonly dataset: DOMStringMap;
    hidden: boolean;
    inert: boolean;
    autofocus: boolean;
    title: string;
    lang: string;
    accessKey: string;
    tabIndex: number;
    readonly offsetWidth: number;
    readonly offsetHeight: number;
    /** Measured from the WINDOW, not from `offsetParent`. */
    readonly offsetTop: number;
    readonly offsetLeft: number;
    readonly offsetParent: Element | null;
    focus(options?: FocusOptions): void;
    blur(): void;
    click(): void;
    cloneNode(deep?: boolean): HTMLElement;
    addEventListener<K extends keyof GlobalEventHandlersEventMap>(
      type: K,
      listener: (ev: GlobalEventHandlersEventMap[K]) => unknown,
      options?: boolean | AddEventListenerOptions,
    ): void;
    addEventListener(
      type: string,
      listener: ((ev: Event) => unknown) | null,
      options?: boolean | AddEventListenerOptions,
    ): void;
    removeEventListener<K extends keyof GlobalEventHandlersEventMap>(
      type: K,
      listener: (ev: GlobalEventHandlersEventMap[K]) => unknown,
      options?: boolean | EventListenerOptions,
    ): void;
    removeEventListener(
      type: string,
      listener: ((ev: Event) => unknown) | null,
      options?: boolean | EventListenerOptions,
    ): void;
  }

  interface SVGElement extends Element {
    readonly style: CSSStyleDeclaration;
    readonly dataset: DOMStringMap;
    tabIndex: number;
    focus(options?: FocusOptions): void;
    blur(): void;
  }
  interface SVGSVGElement extends SVGElement {}
  var SVGElement: { new (): SVGElement; prototype: SVGElement };
  var SVGSVGElement: { new (): SVGSVGElement; prototype: SVGSVGElement };

  interface HTMLInputElement extends HTMLElement {
    value: string;
    defaultValue: string;
    checked: boolean;
    defaultChecked: boolean;
    type: string;
    disabled: boolean;
    required: boolean;
    readOnly: boolean;
    multiple: boolean;
    webkitdirectory: boolean;
    name: string;
    placeholder: string;
    alt: string;
    accept: string;
    min: string;
    max: string;
    step: string;
    pattern: string;
    maxLength: number;
    minLength: number;
    formNoValidate: boolean;
    files: FileList | null;
    /** `type="color"`: the value carries alpha, and the picker shows an alpha slider. */
    alpha: boolean;
    /** `type="color"`: the `colorspace` attribute, read as `"limited-srgb"` or `"display-p3"`. */
    colorSpace: string;
    /** Skathi, `type="color"`: the value is linear light past 1, and the picker has an intensity in stops. */
    hdr: boolean;
    select(): void;
    setSelectionRange(start: number | null, end: number | null, direction?: "forward" | "backward" | "none"): void;
    stepUp(n?: number): void;
    stepDown(n?: number): void;
    /** Needs a user activation (NotAllowedError otherwise). */
    showPicker(): void;
  }
  interface HTMLTextAreaElement extends HTMLElement {
    value: string;
    defaultValue: string;
    disabled: boolean;
    required: boolean;
    readOnly: boolean;
    name: string;
    placeholder: string;
    maxLength: number;
    minLength: number;
    select(): void;
    setSelectionRange(start: number | null, end: number | null, direction?: "forward" | "backward" | "none"): void;
  }
  interface HTMLSelectElement extends HTMLElement {
    value: string;
    selectedIndex: number;
    disabled: boolean;
    required: boolean;
    multiple: boolean;
    name: string;
    showPicker(): void;
  }
  interface HTMLOptionElement extends HTMLElement {
    value: string;
    selected: boolean;
    defaultSelected: boolean;
    disabled: boolean;
  }
  interface HTMLOptGroupElement extends HTMLElement {
    disabled: boolean;
  }
  interface HTMLButtonElement extends HTMLElement {
    disabled: boolean;
    name: string;
    formNoValidate: boolean;
  }
  interface HTMLFieldSetElement extends HTMLElement {
    disabled: boolean;
    name: string;
  }
  interface HTMLFormElement extends HTMLElement {
    noValidate: boolean;
    name: string;
    submit(): void;
    requestSubmit(submitter?: HTMLElement | null): void;
    reset(): void;
  }
  interface HTMLLabelElement extends HTMLElement {
    htmlFor: string;
  }
  interface HTMLOutputElement extends HTMLElement {
    htmlFor: string;
    name: string;
  }
  interface HTMLImageElement extends HTMLElement {
    src: string;
    readonly currentSrc: string;
    alt: string;
    isMap: boolean;
    loading: string;
    referrerPolicy: string;
    readonly complete: boolean;
    readonly naturalWidth: number;
    readonly naturalHeight: number;
    onload: ((ev: Event) => unknown) | null;
    onerror: ((ev: Event) => unknown) | null;
    decode(): Promise<void>;
  }
  interface HTMLAnchorElement extends HTMLElement {}
  interface HTMLDivElement extends HTMLElement {}
  interface HTMLSpanElement extends HTMLElement {}
  interface HTMLParagraphElement extends HTMLElement {}
  interface HTMLHeadingElement extends HTMLElement {}
  interface HTMLPreElement extends HTMLElement {}
  interface HTMLQuoteElement extends HTMLElement {}
  interface HTMLHRElement extends HTMLElement {}
  interface HTMLBRElement extends HTMLElement {}
  interface HTMLUListElement extends HTMLElement {}
  interface HTMLOListElement extends HTMLElement {
    reversed: boolean;
  }
  interface HTMLLIElement extends HTMLElement {}
  interface HTMLDListElement extends HTMLElement {}
  interface HTMLMenuElement extends HTMLElement {}
  interface HTMLTableElement extends HTMLElement {}
  interface HTMLTableCaptionElement extends HTMLElement {}
  interface HTMLTableSectionElement extends HTMLElement {}
  interface HTMLTableRowElement extends HTMLElement {}
  interface HTMLTableCellElement extends HTMLElement {}
  interface HTMLTableColElement extends HTMLElement {}
  interface HTMLDataListElement extends HTMLElement {}
  interface HTMLLegendElement extends HTMLElement {}
  interface HTMLProgressElement extends HTMLElement {}
  interface HTMLMeterElement extends HTMLElement {}
  interface HTMLDetailsElement extends HTMLElement {
    open: boolean;
  }
  /** Only the `open` attribute: `show()`, `close()` and `showModal()` are not implemented. */
  interface HTMLDialogElement extends HTMLElement {
    open: boolean;
  }
  interface HTMLTemplateElement extends HTMLElement {}
  interface HTMLSlotElement extends HTMLElement {
    name: string;
  }
  interface HTMLEmbedElement extends HTMLElement {}
  interface HTMLObjectElement extends HTMLElement {
    name: string;
  }
  interface HTMLMapElement extends HTMLElement {
    name: string;
  }
  interface HTMLAreaElement extends HTMLElement {
    alt: string;
  }
  interface HTMLTimeElement extends HTMLElement {}
  interface HTMLDataElement extends HTMLElement {}
  interface HTMLModElement extends HTMLElement {}
  interface HTMLSourceElement extends HTMLElement {}
  interface HTMLTrackElement extends HTMLElement {}
  interface HTMLPictureElement extends HTMLElement {}
  interface HTMLStyleElement extends HTMLElement {}
  interface HTMLLinkElement extends HTMLElement {
    disabled: boolean;
  }
  interface HTMLScriptElement extends HTMLElement {}
  interface HTMLMetaElement extends HTMLElement {
    name: string;
  }
  interface HTMLTitleElement extends HTMLElement {}
  interface HTMLBaseElement extends HTMLElement {}
  interface HTMLHeadElement extends HTMLElement {}
  interface HTMLBodyElement extends HTMLElement {}
  interface HTMLHtmlElement extends HTMLElement {}

  interface MediaError {
    /** The message is the advisory naming what THIS machine cannot decode. */
    readonly message: string;
    readonly code: number;
  }
  interface HTMLMediaElement extends HTMLElement {
    src: string;
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
    autoplay: boolean;
    controls: boolean;
    readonly error: MediaError | null;
    load(): void;
    play(): Promise<void>;
    pause(): void;
    canPlayType(type: string): "" | "maybe" | "probably";
    readonly HAVE_NOTHING: 0;
    readonly HAVE_METADATA: 1;
    readonly HAVE_CURRENT_DATA: 2;
    readonly HAVE_FUTURE_DATA: 3;
    readonly HAVE_ENOUGH_DATA: 4;
  }
  interface HTMLVideoElement extends HTMLMediaElement {
    readonly videoWidth: number;
    readonly videoHeight: number;
    playsInline: boolean;
  }
  interface HTMLAudioElement extends HTMLMediaElement {}

  /** Media events carry the element's playback state, so a listener needs no read-back. Not web API. */
  interface SkMediaEvent extends Event {
    readonly currentTime: number;
    readonly duration: number;
    readonly readyState: number;
    readonly playbackRate: number;
  }
  /** `<sk-surface>`'s events, read off the event or `detail`. Not web API. */
  interface SkSurfaceEvent extends Event {
    /** `surfaceresize`: the painted content box in physical pixels. `surfaceready`: the first frame's size. */
    readonly width: number;
    readonly height: number;
    /** `surfaceresize` only. */
    readonly devicePixelRatio: number;
    /** `surfacelost`: `"cleared"` (the producer withdrew the frame) or `"device"` (the GPU device was recreated). */
    readonly reason: "cleared" | "device";
  }
  /**
   * `<sk-surface>`'s `surfacedisplaychange`: the display the element is on, first known or changed. Values are in the
   * surface's unit, where 1.0 is SDR white. Not web API.
   */
  interface SkSurfaceDisplayEvent extends Event {
    /** The brightest value the display shows now; 1 on an SDR display. */
    readonly headroom: number;
    /** The brightest it can show (macOS at full brightness). */
    readonly potentialHeadroom: number;
    /** Hz; `null` where unknown (Linux). */
    readonly refreshRate: number | null;
    /** SDR white in nits: Windows in HDR mode, `null` elsewhere. */
    readonly sdrWhiteNits: number | null;
    readonly hdr: boolean;
    /** The window draws HDR now (half-float output), as it does while it shows a half-float surface brighter than SDR white. */
    readonly outputHdr: boolean;
    /** `null` where the platform does not say (Windows, Linux). */
    readonly variableRefresh: boolean | null;
  }
  /** `<lottie>`'s `enterFrame` event; the same numbers are on `detail`. Not web API. */
  interface SkLottieFrameEvent extends Event {
    readonly currentTime: number;
    readonly totalTime: number;
    readonly elapsed: number;
    /** 0 to 1. */
    readonly progress: number;
  }
  /** `<lottie>`'s `loopComplete` event. Not web API. */
  interface SkLottieLoopEvent extends Event {
    readonly currentTime: number;
    readonly totalTime: number;
    readonly currentLoop: number;
  }
  /** `<lottie>`'s `complete` event. Not web API. */
  interface SkLottieCompleteEvent extends Event {
    readonly currentTime: number;
    readonly totalTime: number;
    readonly loops: number;
  }

  interface CanvasGradient {
    addColorStop(offset: number, color: string): void;
  }
  interface TextMetrics {
    readonly width: number;
  }
  interface ImageData {
    readonly width: number;
    readonly height: number;
    readonly data: Uint8ClampedArray;
  }
  interface CanvasRenderingContext2D {
    fillStyle: string | CanvasGradient;
    strokeStyle: string | CanvasGradient;
    lineWidth: number;
    lineCap: "butt" | "round" | "square";
    lineJoin: "miter" | "round" | "bevel";
    globalAlpha: number;
    globalCompositeOperation: string;
    shadowBlur: number;
    shadowColor: string;
    font: string;
    textAlign: "start" | "end" | "left" | "right" | "center";
    textBaseline: "top" | "hanging" | "middle" | "alphabetic" | "ideographic" | "bottom";
    save(): void;
    restore(): void;
    scale(x: number, y: number): void;
    rotate(angle: number): void;
    translate(x: number, y: number): void;
    setTransform(a: number, b: number, c: number, d: number, e: number, f: number): void;
    resetTransform(): void;
    createLinearGradient(x0: number, y0: number, x1: number, y1: number): CanvasGradient;
    createRadialGradient(x0: number, y0: number, r0: number, x1: number, y1: number, r1: number): CanvasGradient;
    clearRect(x: number, y: number, w: number, h: number): void;
    fillRect(x: number, y: number, w: number, h: number): void;
    strokeRect(x: number, y: number, w: number, h: number): void;
    beginPath(): void;
    closePath(): void;
    moveTo(x: number, y: number): void;
    lineTo(x: number, y: number): void;
    quadraticCurveTo(cpx: number, cpy: number, x: number, y: number): void;
    bezierCurveTo(cp1x: number, cp1y: number, cp2x: number, cp2y: number, x: number, y: number): void;
    arc(x: number, y: number, radius: number, startAngle: number, endAngle: number, counterclockwise?: boolean): void;
    ellipse(
      x: number,
      y: number,
      radiusX: number,
      radiusY: number,
      rotation: number,
      startAngle: number,
      endAngle: number,
      counterclockwise?: boolean,
    ): void;
    rect(x: number, y: number, w: number, h: number): void;
    roundRect(x: number, y: number, w: number, h: number, radii?: number | number[]): void;
    fill(): void;
    stroke(): void;
    fillText(text: string, x: number, y: number, maxWidth?: number): void;
    strokeText(text: string, x: number, y: number, maxWidth?: number): void;
    measureText(text: string): TextMetrics;
    setLineDash(segments: number[]): void;
    createImageData(sw: number, sh: number): ImageData;
    getImageData(sx: number, sy: number, sw: number, sh: number): ImageData;
    putImageData(imageData: ImageData, dx: number, dy: number): void;
    drawImage(image: CanvasImageSource, dx: number, dy: number): void;
    drawImage(image: CanvasImageSource, dx: number, dy: number, dw: number, dh: number): void;
    drawImage(
      image: CanvasImageSource,
      sx: number,
      sy: number,
      sw: number,
      sh: number,
      dx: number,
      dy: number,
      dw: number,
      dh: number,
    ): void;
  }
  interface CanvasRenderingContext2DSettings {
    alpha?: boolean;
    willReadFrequently?: boolean;
  }
  interface HTMLCanvasElement extends HTMLElement {
    width: number;
    height: number;
    getContext(contextId: "2d"): CanvasRenderingContext2D | null;
  }

  type PredefinedColorSpace = "display-p3" | "srgb";
  type CanvasImageSource = HTMLImageElement | HTMLVideoElement | HTMLCanvasElement | ImageBitmap | OffscreenCanvas;
  type ImageBitmapSource = HTMLImageElement | HTMLCanvasElement | ImageBitmap | OffscreenCanvas | ImageData | Blob;

  interface ImageBitmap {
    readonly width: number;
    readonly height: number;
    close(): void;
  }
  var ImageBitmap: { new (): ImageBitmap; prototype: ImageBitmap };
  function createImageBitmap(image: ImageBitmapSource): Promise<ImageBitmap>;
  function createImageBitmap(
    image: ImageBitmapSource,
    sx: number,
    sy: number,
    sw: number,
    sh: number,
  ): Promise<ImageBitmap>;

  interface ImageEncodeOptions {
    type?: string;
    quality?: number;
  }
  interface OffscreenCanvasRenderingContext2D extends CanvasRenderingContext2D {
    readonly canvas: OffscreenCanvas;
  }
  /** On the JavaScript thread only: there are no workers to transfer it to. */
  interface OffscreenCanvas extends EventTarget {
    width: number;
    height: number;
    getContext(contextId: "2d", options?: CanvasRenderingContext2DSettings): OffscreenCanvasRenderingContext2D | null;
    getContext(contextId: "webgl" | "webgl2" | "bitmaprenderer"): null;
    convertToBlob(options?: ImageEncodeOptions): Promise<Blob>;
    transferToImageBitmap(): ImageBitmap;
  }
  var OffscreenCanvas: { new (width: number, height: number): OffscreenCanvas; prototype: OffscreenCanvas };

  /** Not provided: WebCodecs is not in Skathi. Declared so the WebGPU types resolve; no value has this type. */
  interface VideoFrame {
    readonly [noVideoFrame]: never;
  }

  interface WindowProxy {
    readonly document: unknown;
    postMessage(message: unknown, targetOrigin: string | { targetOrigin?: string }, transfer?: unknown[]): void;
  }
  interface HTMLIFrameElement extends HTMLElement {
    get sandbox(): DOMTokenList;
    set sandbox(value: string);
    allow: string;
    allowFullscreen: boolean;
    referrerPolicy: string;
    loading: string;
    name: string;
    src: string;
    srcdoc: string;
    readonly contentWindow: WindowProxy | null;
    readonly contentDocument: unknown;
    onload: ((ev: Event) => unknown) | null;
    onerror: ((ev: Event) => unknown) | null;
  }

  interface HTMLElementTagNameMap {
    a: HTMLAnchorElement;
    abbr: HTMLElement;
    address: HTMLElement;
    area: HTMLAreaElement;
    article: HTMLElement;
    aside: HTMLElement;
    audio: HTMLAudioElement;
    b: HTMLElement;
    base: HTMLBaseElement;
    bdi: HTMLElement;
    bdo: HTMLElement;
    blockquote: HTMLQuoteElement;
    body: HTMLBodyElement;
    br: HTMLBRElement;
    button: HTMLButtonElement;
    canvas: HTMLCanvasElement;
    caption: HTMLTableCaptionElement;
    cite: HTMLElement;
    code: HTMLElement;
    col: HTMLTableColElement;
    colgroup: HTMLTableColElement;
    data: HTMLDataElement;
    datalist: HTMLDataListElement;
    dd: HTMLElement;
    del: HTMLModElement;
    details: HTMLDetailsElement;
    dfn: HTMLElement;
    dialog: HTMLDialogElement;
    div: HTMLDivElement;
    dl: HTMLDListElement;
    dt: HTMLElement;
    em: HTMLElement;
    embed: HTMLEmbedElement;
    fieldset: HTMLFieldSetElement;
    figcaption: HTMLElement;
    figure: HTMLElement;
    footer: HTMLElement;
    form: HTMLFormElement;
    h1: HTMLHeadingElement;
    h2: HTMLHeadingElement;
    h3: HTMLHeadingElement;
    h4: HTMLHeadingElement;
    h5: HTMLHeadingElement;
    h6: HTMLHeadingElement;
    head: HTMLHeadElement;
    header: HTMLElement;
    hgroup: HTMLElement;
    hr: HTMLHRElement;
    html: HTMLHtmlElement;
    i: HTMLElement;
    iframe: HTMLIFrameElement;
    img: HTMLImageElement;
    input: HTMLInputElement;
    ins: HTMLModElement;
    kbd: HTMLElement;
    label: HTMLLabelElement;
    legend: HTMLLegendElement;
    li: HTMLLIElement;
    link: HTMLLinkElement;
    lottie: HTMLElement;
    main: HTMLElement;
    map: HTMLMapElement;
    mark: HTMLElement;
    menu: HTMLMenuElement;
    meta: HTMLMetaElement;
    meter: HTMLMeterElement;
    nav: HTMLElement;
    object: HTMLObjectElement;
    ol: HTMLOListElement;
    optgroup: HTMLOptGroupElement;
    option: HTMLOptionElement;
    output: HTMLOutputElement;
    p: HTMLParagraphElement;
    picture: HTMLPictureElement;
    pre: HTMLPreElement;
    progress: HTMLProgressElement;
    q: HTMLQuoteElement;
    s: HTMLElement;
    samp: HTMLElement;
    script: HTMLScriptElement;
    section: HTMLElement;
    select: HTMLSelectElement;
    "sk-sticker": HTMLElement;
    "sk-surface": HTMLElement;
    slot: HTMLSlotElement;
    small: HTMLElement;
    source: HTMLSourceElement;
    span: HTMLSpanElement;
    strong: HTMLElement;
    style: HTMLStyleElement;
    sub: HTMLElement;
    summary: HTMLElement;
    sup: HTMLElement;
    table: HTMLTableElement;
    tbody: HTMLTableSectionElement;
    td: HTMLTableCellElement;
    template: HTMLTemplateElement;
    textarea: HTMLTextAreaElement;
    tfoot: HTMLTableSectionElement;
    th: HTMLTableCellElement;
    thead: HTMLTableSectionElement;
    time: HTMLTimeElement;
    title: HTMLTitleElement;
    tr: HTMLTableRowElement;
    track: HTMLTrackElement;
    u: HTMLElement;
    ul: HTMLUListElement;
    var: HTMLElement;
    video: HTMLVideoElement;
  }
  interface SVGElementTagNameMap {
    svg: SVGSVGElement;
  }

  var HTMLInputElement: { new (): HTMLInputElement; prototype: HTMLInputElement };
  var HTMLTextAreaElement: { new (): HTMLTextAreaElement; prototype: HTMLTextAreaElement };
  var HTMLSelectElement: { new (): HTMLSelectElement; prototype: HTMLSelectElement };
  var HTMLOptionElement: { new (): HTMLOptionElement; prototype: HTMLOptionElement };
  var HTMLOptGroupElement: { new (): HTMLOptGroupElement; prototype: HTMLOptGroupElement };
  var HTMLButtonElement: { new (): HTMLButtonElement; prototype: HTMLButtonElement };
  var HTMLFieldSetElement: { new (): HTMLFieldSetElement; prototype: HTMLFieldSetElement };
  var HTMLFormElement: { new (): HTMLFormElement; prototype: HTMLFormElement };
  var HTMLLabelElement: { new (): HTMLLabelElement; prototype: HTMLLabelElement };
  var HTMLOutputElement: { new (): HTMLOutputElement; prototype: HTMLOutputElement };
  var HTMLImageElement: { new (width?: number, height?: number): HTMLImageElement; prototype: HTMLImageElement };
  var HTMLAnchorElement: { new (): HTMLAnchorElement; prototype: HTMLAnchorElement };
  var HTMLDivElement: { new (): HTMLDivElement; prototype: HTMLDivElement };
  var HTMLSpanElement: { new (): HTMLSpanElement; prototype: HTMLSpanElement };
  var HTMLParagraphElement: { new (): HTMLParagraphElement; prototype: HTMLParagraphElement };
  var HTMLHeadingElement: { new (): HTMLHeadingElement; prototype: HTMLHeadingElement };
  var HTMLPreElement: { new (): HTMLPreElement; prototype: HTMLPreElement };
  var HTMLQuoteElement: { new (): HTMLQuoteElement; prototype: HTMLQuoteElement };
  var HTMLHRElement: { new (): HTMLHRElement; prototype: HTMLHRElement };
  var HTMLBRElement: { new (): HTMLBRElement; prototype: HTMLBRElement };
  var HTMLUListElement: { new (): HTMLUListElement; prototype: HTMLUListElement };
  var HTMLOListElement: { new (): HTMLOListElement; prototype: HTMLOListElement };
  var HTMLLIElement: { new (): HTMLLIElement; prototype: HTMLLIElement };
  var HTMLDListElement: { new (): HTMLDListElement; prototype: HTMLDListElement };
  var HTMLMenuElement: { new (): HTMLMenuElement; prototype: HTMLMenuElement };
  var HTMLTableElement: { new (): HTMLTableElement; prototype: HTMLTableElement };
  var HTMLTableCaptionElement: { new (): HTMLTableCaptionElement; prototype: HTMLTableCaptionElement };
  var HTMLTableSectionElement: { new (): HTMLTableSectionElement; prototype: HTMLTableSectionElement };
  var HTMLTableRowElement: { new (): HTMLTableRowElement; prototype: HTMLTableRowElement };
  var HTMLTableCellElement: { new (): HTMLTableCellElement; prototype: HTMLTableCellElement };
  var HTMLTableColElement: { new (): HTMLTableColElement; prototype: HTMLTableColElement };
  var HTMLDataListElement: { new (): HTMLDataListElement; prototype: HTMLDataListElement };
  var HTMLLegendElement: { new (): HTMLLegendElement; prototype: HTMLLegendElement };
  var HTMLProgressElement: { new (): HTMLProgressElement; prototype: HTMLProgressElement };
  var HTMLMeterElement: { new (): HTMLMeterElement; prototype: HTMLMeterElement };
  var HTMLDetailsElement: { new (): HTMLDetailsElement; prototype: HTMLDetailsElement };
  var HTMLDialogElement: { new (): HTMLDialogElement; prototype: HTMLDialogElement };
  var HTMLTemplateElement: { new (): HTMLTemplateElement; prototype: HTMLTemplateElement };
  var HTMLSlotElement: { new (): HTMLSlotElement; prototype: HTMLSlotElement };
  var HTMLEmbedElement: { new (): HTMLEmbedElement; prototype: HTMLEmbedElement };
  var HTMLObjectElement: { new (): HTMLObjectElement; prototype: HTMLObjectElement };
  var HTMLMapElement: { new (): HTMLMapElement; prototype: HTMLMapElement };
  var HTMLAreaElement: { new (): HTMLAreaElement; prototype: HTMLAreaElement };
  var HTMLTimeElement: { new (): HTMLTimeElement; prototype: HTMLTimeElement };
  var HTMLDataElement: { new (): HTMLDataElement; prototype: HTMLDataElement };
  var HTMLModElement: { new (): HTMLModElement; prototype: HTMLModElement };
  var HTMLSourceElement: { new (): HTMLSourceElement; prototype: HTMLSourceElement };
  var HTMLTrackElement: { new (): HTMLTrackElement; prototype: HTMLTrackElement };
  var HTMLPictureElement: { new (): HTMLPictureElement; prototype: HTMLPictureElement };
  var HTMLStyleElement: { new (): HTMLStyleElement; prototype: HTMLStyleElement };
  var HTMLLinkElement: { new (): HTMLLinkElement; prototype: HTMLLinkElement };
  var HTMLScriptElement: { new (): HTMLScriptElement; prototype: HTMLScriptElement };
  var HTMLMetaElement: { new (): HTMLMetaElement; prototype: HTMLMetaElement };
  var HTMLTitleElement: { new (): HTMLTitleElement; prototype: HTMLTitleElement };
  var HTMLBaseElement: { new (): HTMLBaseElement; prototype: HTMLBaseElement };
  var HTMLHeadElement: { new (): HTMLHeadElement; prototype: HTMLHeadElement };
  var HTMLBodyElement: { new (): HTMLBodyElement; prototype: HTMLBodyElement };
  var HTMLHtmlElement: { new (): HTMLHtmlElement; prototype: HTMLHtmlElement };
  var HTMLMediaElement: {
    new (): HTMLMediaElement;
    prototype: HTMLMediaElement;
    readonly HAVE_NOTHING: 0;
    readonly HAVE_METADATA: 1;
    readonly HAVE_CURRENT_DATA: 2;
    readonly HAVE_FUTURE_DATA: 3;
    readonly HAVE_ENOUGH_DATA: 4;
  };
  var HTMLVideoElement: { new (): HTMLVideoElement; prototype: HTMLVideoElement };
  var HTMLAudioElement: { new (): HTMLAudioElement; prototype: HTMLAudioElement };
  var HTMLCanvasElement: { new (): HTMLCanvasElement; prototype: HTMLCanvasElement };
  var HTMLIFrameElement: { new (): HTMLIFrameElement; prototype: HTMLIFrameElement };

  interface FontFace {
    readonly family: string;
    readonly status: string;
    readonly unicodeRange: string;
    readonly loaded: Promise<FontFace>;
    load(): Promise<FontFace>;
  }
  interface FontFaceSetLoadEvent extends Event {
    readonly fontfaces: readonly FontFace[];
  }
  interface FontFaceSet extends EventTarget, Iterable<FontFace> {
    readonly status: string;
    readonly size: number;
    readonly ready: Promise<FontFaceSet>;
    add(font: FontFace): FontFaceSet;
    has(font: FontFace): boolean;
    check(font: string, text?: string): boolean;
    load(font: string, text?: string): Promise<FontFace[]>;
  }
  var FontFace: { new (family: string, source: string | ArrayBuffer): FontFace; prototype: FontFace };

  type ViewTransitionUpdateCallback = () => unknown;
  interface StartViewTransitionOptions {
    update?: ViewTransitionUpdateCallback | null;
    types?: string[] | null;
  }
  interface ViewTransition {
    readonly updateCallbackDone: Promise<void>;
    readonly ready: Promise<void>;
    readonly finished: Promise<void>;
    readonly types: Set<string>;
    skipTransition(): void;
  }
  var ViewTransition: { prototype: ViewTransition };

  interface Document {
    readonly documentElement: HTMLElement;
    readonly body: HTMLElement;
    readonly activeElement: Element | null;
    readonly fonts: FontFaceSet;
    startViewTransition(callbackOptions?: ViewTransitionUpdateCallback | StartViewTransitionOptions): ViewTransition;
    readonly activeViewTransition: ViewTransition | null;
    readonly fullscreenEnabled: boolean;
    readonly fullscreenElement: Element | null;
    exitFullscreen(): Promise<void>;
    readonly pointerLockElement: Element | null;
    exitPointerLock(): void;
    hasFocus(): boolean;
    /** `"hidden"` while the main window is minimized, hidden, on another virtual desktop or covered by other windows. */
    readonly visibilityState: "visible" | "hidden";
    readonly hidden: boolean;
    /** Inert, and returns `false`, as a browser does when a command cannot be executed; use `navigator.clipboard`. */
    execCommand(commandId: string): boolean;
    getElementById(elementId: string): HTMLElement | null;
    getElementsByName(elementName: string): NodeListOf<HTMLElement>;
    getElementsByClassName(classNames: string): HTMLCollectionOf<Element>;
    getElementsByTagName<K extends keyof HTMLElementTagNameMap>(
      qualifiedName: K,
    ): HTMLCollectionOf<HTMLElementTagNameMap[K]>;
    getElementsByTagName(qualifiedName: string): HTMLCollectionOf<Element>;
    createElement<K extends keyof HTMLElementTagNameMap>(tagName: K): HTMLElementTagNameMap[K];
    createElement(tagName: string): HTMLElement;
    createElementNS(namespaceURI: string | null, qualifiedName: string): Element;
    createTextNode(data: string): Text;
    createComment(data: string): Comment;
    addEventListener<K extends keyof DocumentEventMap>(
      type: K,
      listener: (ev: DocumentEventMap[K]) => unknown,
      options?: boolean | AddEventListenerOptions,
    ): void;
    addEventListener(
      type: string,
      listener: ((ev: Event) => unknown) | null,
      options?: boolean | AddEventListenerOptions,
    ): void;
    removeEventListener<K extends keyof DocumentEventMap>(
      type: K,
      listener: (ev: DocumentEventMap[K]) => unknown,
      options?: boolean | EventListenerOptions,
    ): void;
    removeEventListener(
      type: string,
      listener: ((ev: Event) => unknown) | null,
      options?: boolean | EventListenerOptions,
    ): void;
  }
  // The tree half of Document: in the web it is a mixin, here the members merge in.
  interface Document extends ParentNode {}
  var Document: { new (): Document; prototype: Document };

  interface ResizeObserverEntry {
    readonly target: Element;
    readonly contentRect: DOMRectReadOnly;
  }
  interface ResizeObserver {
    observe(target: Element): void;
    unobserve(target: Element): void;
    disconnect(): void;
  }
  type ResizeObserverCallback = (entries: ResizeObserverEntry[], observer: ResizeObserver) => void;
  var ResizeObserver: { new (callback: ResizeObserverCallback): ResizeObserver; prototype: ResizeObserver };

  interface IntersectionObserverEntry {
    readonly target: Element;
    readonly isIntersecting: boolean;
    readonly intersectionRatio: number;
    readonly boundingClientRect: DOMRectReadOnly;
  }
  interface IntersectionObserver {
    observe(target: Element): void;
    unobserve(target: Element): void;
    disconnect(): void;
  }
  type IntersectionObserverCallback = (entries: IntersectionObserverEntry[], observer: IntersectionObserver) => void;
  var IntersectionObserver: {
    new (callback: IntersectionObserverCallback): IntersectionObserver;
    prototype: IntersectionObserver;
  };

  interface ClipboardItem {
    readonly types: readonly string[];
    getType(type: string): Promise<Blob>;
  }
  var ClipboardItem: { new (items: Record<string, Blob>): ClipboardItem; prototype: ClipboardItem };

  interface MediaStreamConstraints {
    audio?: boolean;
    video?: boolean;
  }
  interface MediaTrackSettings {
    readonly echoCancellation?: boolean;
    readonly noiseSuppression?: boolean;
    readonly autoGainControl?: boolean;
    readonly channelCount?: number;
  }
  interface MediaStreamTrack extends EventTarget {
    readonly kind: string;
    readonly readyState: string;
    readonly label: string;
    getSettings(): MediaTrackSettings;
    stop(): void;
  }
  interface MediaStream extends EventTarget {
    getAudioTracks(): MediaStreamTrack[];
    getTracks(): MediaStreamTrack[];
  }
  interface MediaDeviceInfo {
    readonly kind: string;
    readonly deviceId: string;
    readonly label: string;
  }
  interface MediaDevices extends EventTarget {
    getUserMedia(constraints?: MediaStreamConstraints): Promise<MediaStream>;
    enumerateDevices(): Promise<MediaDeviceInfo[]>;
  }
}

declare global {
  /** Hot module replacement under `sk dev`, as Vite spells it. */
  interface SkHotContext {
    readonly data: Record<string, unknown>;
    accept(cb?: (module: unknown) => void): void;
    dispose(cb: (data: Record<string, unknown>) => void): void;
    prune(cb: (data: Record<string, unknown>) => void): void;
    invalidate(message?: string): void;
    decline(): void;
  }
  interface ImportMeta {
    /** Undefined in a build. */
    readonly hot?: SkHotContext;
    /** The base of `new URL("./file", import.meta.url)`, which the builder resolves to the file's URL in the app. */
    readonly url: string;
  }
}

declare global {
  interface Storage {
    readonly length: number;
    key(index: number): string | null;
    getItem(key: string): string | null;
    /** Typed strictly so the coercion is written down at the call site rather than being invisible. */
    setItem(key: string, value: string): void;
    removeItem(key: string): void;
    clear(): void;
  }
  var localStorage: Storage;
  /** Per-run, never written anywhere. */
  var sessionStorage: Storage;
}

declare global {
  interface FileList extends Iterable<File> {
    readonly length: number;
    item(index: number): File | null;
    readonly [index: number]: File;
  }
  var FileList: { new (): FileList; prototype: FileList };

  interface DataTransferItem {
    readonly kind: "string" | "file" | "";
    readonly type: string;
    getAsString(callback: ((data: string) => void) | null): void;
    getAsFile(): File | null;
    webkitGetAsEntry(): FileSystemEntry | null;
    getAsFileSystemHandle?(): Promise<FileSystemHandle | null>;
  }
  interface FileSystem {
    readonly name: string;
    readonly root: FileSystemDirectoryEntry;
  }
  interface FileSystemEntry {
    readonly filesystem: FileSystem;
    readonly fullPath: string;
    readonly name: string;
    readonly isFile: boolean;
    readonly isDirectory: boolean;
    getParent(success?: (entry: FileSystemDirectoryEntry) => void, error?: (error: DOMException) => void): void;
  }
  interface FileSystemFileEntry extends FileSystemEntry {
    file(success: (file: File) => void, error?: (error: DOMException) => void): void;
  }
  interface FileSystemDirectoryEntry extends FileSystemEntry {
    createReader(): FileSystemDirectoryReader;
    getFile(
      path?: string,
      options?: { create?: boolean; exclusive?: boolean },
      success?: (entry: FileSystemFileEntry) => void,
      error?: (error: DOMException) => void,
    ): void;
    getDirectory(
      path?: string,
      options?: { create?: boolean; exclusive?: boolean },
      success?: (entry: FileSystemDirectoryEntry) => void,
      error?: (error: DOMException) => void,
    ): void;
  }
  interface FileSystemDirectoryReader {
    readEntries(success: (entries: FileSystemEntry[]) => void, error?: (error: DOMException) => void): void;
  }
  var FileSystem: { new (): FileSystem; prototype: FileSystem };
  var FileSystemEntry: { new (): FileSystemEntry; prototype: FileSystemEntry };
  var FileSystemFileEntry: { new (): FileSystemFileEntry; prototype: FileSystemFileEntry };
  var FileSystemDirectoryEntry: { new (): FileSystemDirectoryEntry; prototype: FileSystemDirectoryEntry };
  var FileSystemDirectoryReader: { new (): FileSystemDirectoryReader; prototype: FileSystemDirectoryReader };

  interface DataTransferItemList extends Iterable<DataTransferItem> {
    readonly length: number;
    readonly [index: number]: DataTransferItem;
    add(data: string, type: string): DataTransferItem | null;
    add(data: File): DataTransferItem | null;
    remove(index: number): void;
    clear(): void;
  }
  interface DataTransfer {
    dropEffect: "none" | "copy" | "link" | "move";
    effectAllowed: "none" | "copy" | "copyLink" | "copyMove" | "link" | "linkMove" | "move" | "all" | "uninitialized";
    readonly items: DataTransferItemList;
    readonly types: readonly string[];
    readonly files: FileList;
    getData(format: string): string;
    setData(format: string, data: string): void;
    clearData(format?: string): void;
    setDragImage(image: Element, x: number, y: number): void;
  }
  var DataTransfer: { new (): DataTransfer; prototype: DataTransfer };

  interface DragEvent extends MouseEvent {
    readonly dataTransfer: DataTransfer | null;
  }
  var DragEvent: {
    new (type: string, eventInitDict?: MouseEventInit & { dataTransfer?: DataTransfer | null }): DragEvent;
    prototype: DragEvent;
  };

  interface CloseEvent extends Event {
    readonly code: number;
    readonly reason: string;
    readonly wasClean: boolean;
  }
  interface MessageEvent<T = unknown> extends Event {
    readonly data: T;
    readonly origin: string;
    readonly lastEventId: string;
  }
  var CloseEvent: {
    new (type: string, eventInitDict?: EventInit & { code?: number; reason?: string; wasClean?: boolean }): CloseEvent;
    prototype: CloseEvent;
  };
  var MessageEvent: {
    new <T = unknown>(type: string, eventInitDict?: EventInit & { data?: T; origin?: string }): MessageEvent<T>;
    prototype: MessageEvent;
  };

  interface WebSocket extends EventTarget {
    readonly readyState: number;
    readonly bufferedAmount: number;
    readonly protocol: string;
    readonly url: string;
    binaryType: "blob" | "arraybuffer";
    onopen: ((ev: Event) => unknown) | null;
    onerror: ((ev: Event) => unknown) | null;
    onclose: ((ev: CloseEvent) => unknown) | null;
    onmessage: ((ev: MessageEvent<string | Blob | ArrayBuffer>) => unknown) | null;
    send(data: string | ArrayBuffer | ArrayBufferView | Blob): void;
    close(code?: number, reason?: string): void;
    readonly CONNECTING: 0;
    readonly OPEN: 1;
    readonly CLOSING: 2;
    readonly CLOSED: 3;
  }
  var WebSocket: {
    new (url: string | URL, protocols?: string | string[]): WebSocket;
    prototype: WebSocket;
    readonly CONNECTING: 0;
    readonly OPEN: 1;
    readonly CLOSING: 2;
    readonly CLOSED: 3;
  };
}

// Every picker needs a user activation, else SecurityError.

declare global {
  interface FileSystemHandlePermissionDescriptor {
    mode?: "read" | "readwrite";
  }
  interface FileSystemHandle {
    readonly kind: "file" | "directory";
    readonly name: string;
    isSameEntry(other: FileSystemHandle): Promise<boolean>;
    queryPermission(descriptor?: FileSystemHandlePermissionDescriptor): Promise<PermissionState>;
    requestPermission(descriptor?: FileSystemHandlePermissionDescriptor): Promise<PermissionState>;
  }
  interface FileSystemCreateWritableOptions {
    keepExistingData?: boolean;
  }
  interface FileSystemFileHandle extends FileSystemHandle {
    readonly kind: "file";
    getFile(): Promise<File>;
    createWritable(options?: FileSystemCreateWritableOptions): Promise<FileSystemWritableFileStream>;
  }
  interface FileSystemDirectoryHandle
    extends FileSystemHandle, AsyncIterable<[string, FileSystemFileHandle | FileSystemDirectoryHandle]> {
    readonly kind: "directory";
    getFileHandle(name: string, options?: { create?: boolean }): Promise<FileSystemFileHandle>;
    getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<FileSystemDirectoryHandle>;
    removeEntry(name: string, options?: { recursive?: boolean }): Promise<void>;
    resolve(possibleDescendant: FileSystemHandle): Promise<string[] | null>;
    entries(): AsyncIterableIterator<[string, FileSystemFileHandle | FileSystemDirectoryHandle]>;
    keys(): AsyncIterableIterator<string>;
    values(): AsyncIterableIterator<FileSystemFileHandle | FileSystemDirectoryHandle>;
  }
  interface WriteParams {
    type: "write" | "seek" | "truncate";
    position?: number | null;
    size?: number | null;
    data?: BufferSource | Blob | string | null;
  }
  type FileSystemWriteChunkType = BufferSource | Blob | string | WriteParams;
  interface FileSystemWritableFileStream {
    readonly locked: boolean;
    write(data: FileSystemWriteChunkType): Promise<void>;
    seek(position: number): Promise<void>;
    truncate(size: number): Promise<void>;
    close(): Promise<void>;
    abort(reason?: unknown): Promise<void>;
  }
  // Constructing one throws "Illegal constructor", but `x instanceof FileSystemFileHandle` needs the signature.
  var FileSystemHandle: { new (): FileSystemHandle; prototype: FileSystemHandle };
  var FileSystemFileHandle: { new (): FileSystemFileHandle; prototype: FileSystemFileHandle };
  var FileSystemDirectoryHandle: { new (): FileSystemDirectoryHandle; prototype: FileSystemDirectoryHandle };
  var FileSystemWritableFileStream: {
    new (): FileSystemWritableFileStream;
    prototype: FileSystemWritableFileStream;
  };

  type WellKnownDirectory = "desktop" | "documents" | "downloads" | "music" | "pictures" | "videos";
  interface FilePickerAcceptType {
    description?: string;
    accept: Record<string, string | string[]>;
  }
  interface FilePickerOptions {
    id?: string;
    startIn?: WellKnownDirectory | FileSystemHandle;
    types?: FilePickerAcceptType[];
    excludeAcceptAllOption?: boolean;
  }
  interface OpenFilePickerOptions extends FilePickerOptions {
    multiple?: boolean;
  }
  interface SaveFilePickerOptions extends FilePickerOptions {
    suggestedName?: string;
  }
  interface DirectoryPickerOptions {
    id?: string;
    startIn?: WellKnownDirectory | FileSystemHandle;
    mode?: "read" | "readwrite";
  }
  function showOpenFilePicker(options?: OpenFilePickerOptions): Promise<FileSystemFileHandle[]>;
  function showSaveFilePicker(options?: SaveFilePickerOptions): Promise<FileSystemFileHandle>;
  function showDirectoryPicker(options?: DirectoryPickerOptions): Promise<FileSystemDirectoryHandle>;

  interface ProgressEventInit extends EventInit {
    lengthComputable?: boolean;
    loaded?: number;
    total?: number;
  }
  interface ProgressEvent extends Event {
    readonly lengthComputable: boolean;
    readonly loaded: number;
    readonly total: number;
  }
  var ProgressEvent: { new (type: string, eventInitDict?: ProgressEventInit): ProgressEvent; prototype: ProgressEvent };

  interface FileReader extends EventTarget {
    readonly readyState: 0 | 1 | 2;
    readonly result: string | ArrayBuffer | null;
    readonly error: DOMException | null;
    readonly EMPTY: 0;
    readonly LOADING: 1;
    readonly DONE: 2;
    readAsArrayBuffer(blob: Blob): void;
    readAsBinaryString(blob: Blob): void;
    /** `encoding`, else the Blob type's charset, else UTF-8; a BOM wins over all three. */
    readAsText(blob: Blob, encoding?: string): void;
    readAsDataURL(blob: Blob): void;
    abort(): void;
    onloadstart: ((ev: ProgressEvent) => unknown) | null;
    onprogress: ((ev: ProgressEvent) => unknown) | null;
    onload: ((ev: ProgressEvent) => unknown) | null;
    onerror: ((ev: ProgressEvent) => unknown) | null;
    onabort: ((ev: ProgressEvent) => unknown) | null;
    onloadend: ((ev: ProgressEvent) => unknown) | null;
  }
  var FileReader: {
    new (): FileReader;
    prototype: FileReader;
    readonly EMPTY: 0;
    readonly LOADING: 1;
    readonly DONE: 2;
  };
}

// Simple dialogs and notifications have no Skathi implementation: each advises once and answers as
// a browser does when the user dismisses or denies.

declare global {
  /** @deprecated Not supported in Skathi: shows nothing. Show the message in the app's own UI. */
  function alert(message?: unknown): void;
  /** @deprecated Not supported in Skathi: answers `false`. Ask in the app's own UI. */
  function confirm(message?: string): boolean;
  /** @deprecated Not supported in Skathi: answers `null`. Ask for the text in the app's own UI. */
  function prompt(message?: string, _default?: string): string | null;

  type NotificationPermission = "default" | "denied" | "granted";
  interface NotificationOptions {
    body?: string;
    tag?: string;
    icon?: string;
    data?: unknown;
    silent?: boolean | null;
    requireInteraction?: boolean;
  }
  /** @deprecated Not supported in Skathi: nothing is shown. Show the message in the app's own UI. */
  interface Notification extends EventTarget {
    readonly title: string;
    readonly body: string;
    readonly tag: string;
    readonly icon: string;
    readonly data: unknown;
    onclick: ((ev: Event) => unknown) | null;
    onclose: ((ev: Event) => unknown) | null;
    onerror: ((ev: Event) => unknown) | null;
    onshow: ((ev: Event) => unknown) | null;
    close(): void;
  }
  /** @deprecated Not supported in Skathi: nothing is shown and permission is always `"denied"`. Show the message in the app's own UI. */
  var Notification: {
    new (title: string, options?: NotificationOptions): Notification;
    prototype: Notification;
    readonly permission: NotificationPermission;
    requestPermission(
      deprecatedCallback?: (permission: NotificationPermission) => void,
    ): Promise<NotificationPermission>;
  };
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

declare global {
  var window: typeof globalThis;
  var onerror: ((...args: unknown[]) => unknown) | null;
  function atob(data: string): string;
  function btoa(data: string): string;
  function getComputedStyle(elt: Element): CSSStyleDeclaration;

  interface CompositionEvent extends UIEvent {
    readonly data: string;
  }
  interface ErrorEvent extends Event {
    readonly message: string;
    readonly filename: string;
    readonly lineno: number;
    readonly colno: number;
    readonly error: unknown;
  }
  var CompositionEvent: {
    new (type: string, eventInitDict?: UIEventInit & { data?: string }): CompositionEvent;
    prototype: CompositionEvent;
  };
  var ErrorEvent: {
    new (type: string, eventInitDict?: EventInit & { message?: string; error?: unknown }): ErrorEvent;
    prototype: ErrorEvent;
  };
}
