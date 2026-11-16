// `<SkWindow>`: a second OS window that paints part of THIS component tree. The subtree stays in
// this tree, component instance and reactive graph; only its paint moves, so nothing is
// remounted or serialized and a click in the pop-out runs the same handler.
//
//     <SkWindow v-if="open" title="Palette" :bounds="{ width: 320 }" @close="open = false">
//       <Palette v-model="brush" />
//     </SkWindow>
//
// A component rather than `<Teleport to="sk:window">`: like `window.open()` it supplies the target
// element, so `<Teleport :to="pal.el">` composes onto a `SkWindow` as onto a popup's body.
// `createSkWindowPopout` is the host side, shared with the Svelte component.

import {
  defineComponent,
  h,
  mergeProps,
  onBeforeUnmount,
  onMounted,
  onUnmounted,
  reactive,
  ref,
  toRefs,
  watch,
} from "@vue/runtime-core";
import type { PropType } from "@vue/runtime-core";

/** The container node `<SkWindow>` renders as the pop-out's root. Structural: importing runtime.ts's `MirrorElement` would be circular. */
export interface SkWindowTarget {
  readonly __skId: number;
}

export interface SkWindowBounds {
  x?: number;
  y?: number;
  width?: number;
  height?: number;
}

export interface SkWindowMetrics {
  width: number;
  height: number;
  scale: number;
}

export interface SkWindowShown extends SkWindowMetrics {
  firstFrameMs: number;
}

export interface SkWindowRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** `<SkWindow>`'s props, the same in Vue and Svelte; an absent one takes its default. */
export interface SkWindowOptions {
  /** The OS caption text. Default `"Skathi"`. Live: changing it renames the window. */
  title?: string | undefined;
  /** Where the window opens, CSS px; without `x`/`y`, centred on the main window's monitor. Live on Windows: a change moves or resizes the open window. */
  bounds?: SkWindowBounds | undefined;
  /** `"default"`: OS caption. `"hidden"`: the app paints its own `window-drag: move` region. Creation-time only. */
  chrome?: "default" | "hidden" | undefined;
  /** `mica` | `mica-alt` | `acrylic` | `transparent` | `none`. Default: the app's startup backdrop. Live. */
  backdrop?: string | undefined;
  /** `HWND_TOPMOST`. Live. */
  alwaysOnTop?: boolean | undefined;
  /** Default true. Creation-time only. */
  resizable?: boolean | undefined;
  /** Minimum client width, CSS px. Creation-time only. */
  minWidth?: number | undefined;
  /** Minimum client height, CSS px. Creation-time only. */
  minHeight?: number | undefined;
  /** `env(titlebar-area-height)` inside this window, for `chrome: "hidden"`. Default 32. Live. */
  titleBarHeight?: number | undefined;
  /** Take the keyboard on show? Default false: a palette that grabs focus steals the caret from the document. Live: read at each show. */
  activate?: boolean | undefined;
  /** Default true. Live. `false` keeps the window created and painted but hidden, so the first `true` is a frame, not a creation. */
  visible?: boolean | undefined;
  /** `"monitor"`: frameless, covers the whole monitor the main window is on (taskbar included); `bounds` ignored. Creation-time only. */
  cover?: "monitor" | undefined;
  /** Default true. `false`: owned by the main window: no taskbar button or Alt-Tab entry, stays above it. Creation-time only. */
  taskbar?: boolean | undefined;
  /** Keep the full image decode while shown here (no reduced copy for small draws). Live. */
  fullResolution?: boolean | undefined;
  /** The mouse passes THROUGH the whole window to whatever is under it. Live. */
  ignoreMouseEvents?: boolean | undefined;
  /** Default true. `false`: never activated, no taskbar or Alt-Tab entry, no keyboard. Creation-time only. */
  focusable?: boolean | undefined;
  /** Excluded from screen capture. Live. */
  contentProtection?: boolean | undefined;
  /** With `cover="monitor"`: monitor index; absent means the main window's. Live: a shown window moves. */
  monitor?: number | undefined;
}

/** What a pop-out reports: Vue's `@open`…`@hide`, Svelte's `onopen`…`onhide`. */
export interface SkWindowEvents {
  /** The OS window exists. Carries its window id, what devtools `--window` names. */
  open(windowId: number): void;
  /** The window closed: by the user or the OS, or because the component unmounts (then synchronously, before it does). */
  close(): void;
  focus(): void;
  blur(): void;
  resize(metrics: SkWindowMetrics): void;
  /** The window is on screen with its first frame drawn: once after it opens, and again after each hide. */
  show(shown: SkWindowShown): void;
  hide(): void;
}

/** A pop-out's window id and its own viewport in CSS px; `appWindow`, `innerWidth` and `matchMedia` keep meaning the main window. */
export interface SkWindowState extends SkWindowMetrics {
  windowId: number;
}

/** The host side of one mounted `<SkWindow>`, shared by the Vue and Svelte components. */
export interface SkWindowPopout {
  /** The props as the host reads them. Reads every prop, so a reactive caller tracks them all. */
  options(): string;
  /** Opens the window painting `node`, once it is mounted. */
  open(node: SkWindowTarget): void;
  /** Sends the props if they changed since the last send; nothing before the window exists. */
  sync(): void;
  /** Closes the window while `node` still exists, reporting `close` if the window had opened. */
  close(): void;
  /** Where an element of the MAIN window is, in this window's CSS px relative to its root; null if either is unpainted. */
  mapFromMain(target: SkWindowTarget | null | undefined): SkWindowRect | null;
}

/** Props fixed at window creation; changing one on a live window advises and does nothing (close and re-open via `v-if` or a `:key`). */
const CREATION_TIME_ONLY = ["chrome", "resizable", "minWidth", "minHeight", "cover", "taskbar", "focusable"] as const;

/** Symbol prop marking a window's painted root: Vue's `for…in` skips symbols so it never reaches the host, yet `createElement` registers the element before its children mount. */
export const WINDOW_ROOT: unique symbol = Symbol("sk-window-root");

/** Painted roots of the mounted `<SkWindow>`s; a node under one belongs to that window's document. Empty without a pop-out. */
export const windowRoots = new Set<object>();

/** Marks `node` as a pop-out's painted root until the returned function runs. */
export function registerWindowRoot(node: SkWindowTarget): () => void {
  windowRoots.add(node);
  return () => void windowRoots.delete(node);
}

const DEFAULTS = {
  title: "Skathi",
  chrome: "default",
  alwaysOnTop: false,
  resizable: true,
  titleBarHeight: 32,
  activate: false,
  visible: true,
  taskbar: true,
  fullResolution: false,
  ignoreMouseEvents: false,
  focusable: true,
  contentProtection: false,
} as const;

interface OpenOptions extends SkWindowBounds {
  title: string;
  chrome: string;
  backdrop?: string;
  alwaysOnTop: boolean;
  resizable: boolean;
  minWidth?: number;
  minHeight?: number;
  titleBarHeight: number;
  activate: boolean;
  visible: boolean;
  cover?: string;
  taskbar: boolean;
  fullResolution: boolean;
  ignoreMouseEvents: boolean;
  focusable: boolean;
  contentProtection: boolean;
  monitor?: number;
}

export function createSkWindowPopout(
  props: () => SkWindowOptions,
  on: SkWindowEvents,
  state: SkWindowState,
): SkWindowPopout {
  let node: SkWindowTarget | null = null;
  let windowId = 0;
  let closed = false;
  let sent = "";

  const options = (): string => {
    const p = props();
    const o: OpenOptions = {
      ...(p.bounds ?? {}),
      title: p.title ?? DEFAULTS.title,
      chrome: p.chrome ?? DEFAULTS.chrome,
      alwaysOnTop: p.alwaysOnTop ?? DEFAULTS.alwaysOnTop,
      resizable: p.resizable ?? DEFAULTS.resizable,
      activate: p.activate ?? DEFAULTS.activate,
      titleBarHeight: p.titleBarHeight ?? DEFAULTS.titleBarHeight,
      visible: p.visible ?? DEFAULTS.visible,
      taskbar: p.taskbar ?? DEFAULTS.taskbar,
      fullResolution: p.fullResolution ?? DEFAULTS.fullResolution,
      ignoreMouseEvents: p.ignoreMouseEvents ?? DEFAULTS.ignoreMouseEvents,
      focusable: p.focusable ?? DEFAULTS.focusable,
      contentProtection: p.contentProtection ?? DEFAULTS.contentProtection,
    };
    if (p.monitor !== undefined) o.monitor = p.monitor;
    if (p.backdrop !== undefined) o.backdrop = p.backdrop;
    if (p.minWidth !== undefined) o.minWidth = p.minWidth;
    if (p.minHeight !== undefined) o.minHeight = p.minHeight;
    if (p.cover !== undefined) o.cover = p.cover;
    return JSON.stringify(o);
  };

  const sync = () => {
    if (closed || !node || !windowId) return;
    const now = options();
    if (now === sent) return;
    sent = now;
    __sk_window_set_options(node.__skId, now);
  };

  const metrics = (): SkWindowMetrics => ({ width: state.width, height: state.height, scale: state.scale });

  const onEvent = (kind: string, detailJson: string) => {
    let detail: Record<string, number> = {};
    try {
      detail = JSON.parse(detailJson) as Record<string, number>;
    } catch {
      /* the host always sends an object; an unparsable one is not worth throwing over */
    }
    switch (kind) {
      case "open":
        windowId = detail.windowId ?? 0;
        state.windowId = windowId;
        sync();
        on.open(windowId);
        break;
      case "show":
        if (detail.width) state.width = detail.width;
        if (detail.height) state.height = detail.height;
        if (detail.scale) state.scale = detail.scale;
        on.show({ firstFrameMs: detail.firstFrameMs ?? 0, ...metrics() });
        break;
      case "hide":
        on.hide();
        break;
      case "focus":
        on.focus();
        break;
      case "blur":
        on.blur();
        break;
      case "resize":
        state.width = detail.width ?? 0;
        state.height = detail.height ?? 0;
        state.scale = detail.scale ?? 1;
        on.resize(metrics());
        break;
    }
  };

  return {
    options,
    sync,
    open(target) {
      if (node) return;
      node = target;
      // The host defers the open by one effect cycle (it nests in the primary window's update), so
      // no handle is returned; the window id comes with `open`. Un-painting is immediate: a node in
      // two element trees for one frame aliases gpui's scroll state and hitboxes.
      sent = options();
      __sk_window_open(
        target.__skId,
        sent,
        () => {
          const already = closed;
          closed = true;
          windowId = 0;
          state.windowId = 0;
          if (!already) on.close();
        },
        onEvent,
      );
    },
    close() {
      if (closed || !node) return;
      closed = true;
      __sk_window_close(node.__skId);
      if (windowId) on.close();
    },
    mapFromMain(target) {
      if (closed || !node || !target || typeof __sk_window_map_rect !== "function") return null;
      const raw = __sk_window_map_rect(node.__skId, target.__skId);
      if (typeof raw !== "string") return null;
      const [x, y, w, h] = JSON.parse(raw) as number[];
      return { x: x!, y: y!, width: w!, height: h! };
    },
  };
}

export const SkWindow = /*@__PURE__*/ defineComponent({
  name: "SkWindow",
  inheritAttrs: false,
  props: {
    /** The OS caption text, default `"Skathi"`. Live: changing it renames the window. */
    title: { type: String, default: undefined },
    /** Where the window opens, CSS px; without `x`/`y`, centred on the main window's monitor. Live on Windows: a change moves or resizes the open window. */
    bounds: { type: Object as PropType<SkWindowBounds>, default: undefined },
    /** `"default"` (the default): OS caption. `"hidden"`: the app paints its own `window-drag: move` region. Creation-time only. */
    chrome: { type: String as PropType<"default" | "hidden">, default: undefined },
    /** `mica` | `mica-alt` | `acrylic` | `transparent` | `none`. Default: the app's startup backdrop. Live. */
    backdrop: { type: String, default: undefined },
    /** `HWND_TOPMOST`. Live. */
    alwaysOnTop: { type: Boolean, default: undefined },
    /** Default true. Creation-time only. */
    resizable: { type: Boolean, default: undefined },
    /** Minimum client width, CSS px. Creation-time only. */
    minWidth: { type: Number, default: undefined },
    /** Minimum client height, CSS px. Creation-time only. */
    minHeight: { type: Number, default: undefined },
    /** `env(titlebar-area-height)` inside this window, for `chrome: "hidden"`. Default 32. Live. */
    titleBarHeight: { type: Number, default: undefined },
    /** Take the keyboard on show? Default false: a palette that grabs focus steals the caret from the document. Live: read at each show. */
    activate: { type: Boolean, default: undefined },
    /** Default true. Live. `false` keeps the window created and painted but hidden, so the first `true` is a frame, not a creation. A hide returns activation to the main window. */
    visible: { type: Boolean, default: undefined },
    /** `"monitor"`: frameless, covers the whole monitor the main window is on (taskbar included); `bounds` ignored. Creation-time only. */
    cover: { type: String as PropType<"monitor">, default: undefined },
    /** Default true. `false`: owned by the main window: no taskbar button or Alt-Tab entry, stays above it, minimises with it. Creation-time only. */
    taskbar: { type: Boolean, default: undefined },
    /** Keep the full image decode while shown here (no reduced copy for small draws). Live. */
    fullResolution: { type: Boolean, default: undefined },
    /** The mouse passes THROUGH the whole window to whatever is under it (Electron `setIgnoreMouseEvents(true)`). Live. */
    ignoreMouseEvents: { type: Boolean, default: undefined },
    /** Default true. `false`: never activated, no taskbar or Alt-Tab entry, no keyboard (Electron `focusable: false`). Creation-time only. */
    focusable: { type: Boolean, default: undefined },
    /** Excluded from screen capture (`WDA_EXCLUDEFROMCAPTURE`). Live. */
    contentProtection: { type: Boolean, default: undefined },
    /** With `cover="monitor"`: monitor index in `EnumDisplayMonitors` order; absent means the main window's. Live: a shown window moves. */
    monitor: { type: Number as PropType<number | undefined>, default: undefined },
  },
  emits: {
    /** The OS window exists. Carries its gpui window id — what devtools `--window` names. */
    open: (_windowId: number) => true,
    /** The window closed: by the user or the OS, or because the component unmounts (then synchronously, before it does). */
    close: () => true,
    focus: () => true,
    blur: () => true,
    resize: (_metrics: SkWindowMetrics) => true,
    /** The window is on screen with its first frame drawn: once after it opens, and again after each hide. */
    show: (_shown: SkWindowShown) => true,
    hide: () => true,
  },
  setup(props, { slots, emit, expose, attrs }) {
    const el = ref<SkWindowTarget | null>(null);
    const state = reactive<SkWindowState>({ windowId: 0, width: 0, height: 0, scale: 1 });
    const popout = createSkWindowPopout(
      () => props,
      {
        open: (windowId) => emit("open", windowId),
        close: () => emit("close"),
        focus: () => emit("focus"),
        blur: () => emit("blur"),
        resize: (metrics) => emit("resize", metrics),
        show: (shown) => emit("show", shown),
        hide: () => emit("hide"),
      },
      state,
    );

    expose({ el, ...toRefs(state), mapFromMain: popout.mapFromMain });

    onMounted(() => {
      if (el.value) popout.open(el.value);
    });
    // `onBeforeUnmount`, not `onUnmounted`: the node must exist when the close is requested; after
    // `onUnmounted` Vue has called `__sk_remove`. `close` is emitted here because the host's close
    // completes after the unmount, when Vue drops emits.
    onBeforeUnmount(() => popout.close());
    // Leaves the registry after the content unmounts, so children's unmount hooks still see their window.
    let root: object | null = null;
    onMounted(() => (root = el.value));
    onUnmounted(() => {
      if (root) windowRoots.delete(root);
    });

    watch(popout.options, popout.sync);

    // The pop-out's painted root: the host gives it the main window's root treatment (100%x100%, scroll, focus handle) so
    // content fills the window and an `<input>` can hold focus. It renders in this tree between mount and the deferred
    // open (`.sk-window-root`). Attrs are merged here because Vue's fall-through `for…in` would drop `WINDOW_ROOT`.
    return () => {
      const root: Record<string | symbol, unknown> = mergeProps({ ref: el, class: "sk-window-root" }, attrs);
      root[WINDOW_ROOT] = true;
      return h("div", root, slots.default?.());
    };
  },
});

export const SK_WINDOW_CREATION_TIME_PROPS: readonly string[] = CREATION_TIME_ONLY;
