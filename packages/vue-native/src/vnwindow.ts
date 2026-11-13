// `<VnWindow>`: a second OS window that paints part of THIS component tree. The subtree stays in
// this tree, component instance and reactive graph; only its paint moves, so nothing is
// remounted or serialized and a click in the pop-out runs the same handler.
//
//     <VnWindow v-if="open" title="Palette" :bounds="{ width: 320 }" @close="open = false">
//       <Palette v-model="brush" />
//     </VnWindow>
//
// A component rather than `<Teleport to="vn:window">`: like `window.open()` it supplies the target
// element, so `<Teleport :to="pal.el">` composes onto a `VnWindow` as onto a popup's body.

import { defineComponent, h, mergeProps, onBeforeUnmount, onMounted, onUnmounted, ref, watch } from "@vue/runtime-core";
import type { PropType } from "@vue/runtime-core";

/** The container node `<VnWindow>` renders as the pop-out's root. Structural: importing runtime.ts's `MirrorElement` would be circular. */
export interface VnWindowTarget {
  readonly __vnId: number;
}

export interface VnWindowBounds {
  x?: number;
  y?: number;
  width?: number;
  height?: number;
}

export interface VnWindowMetrics {
  width: number;
  height: number;
  scale: number;
}

export interface VnWindowShown extends VnWindowMetrics {
  firstFrameMs: number;
}

export interface VnWindowRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Props gpui fixes at window creation; changing one on a live window advises and does nothing (close and re-open via `v-if`). */
const CREATION_TIME_ONLY = [
  "chrome",
  "backdrop",
  "resizable",
  "minWidth",
  "minHeight",
  "cover",
  "taskbar",
  "focusable",
] as const;

/** Symbol prop marking a window's painted root: Vue's `for…in` skips symbols so it never reaches the host, yet `createElement` registers the element before its children mount. */
export const WINDOW_ROOT: unique symbol = Symbol("vn-window-root");

/** Painted roots of the mounted `<VnWindow>`s; a node under one belongs to that window's document. Empty without a pop-out. */
export const windowRoots = new Set<object>();

interface OpenOptions extends VnWindowBounds {
  title: string;
  chrome: string;
  backdrop?: string;
  alwaysOnTop: boolean;
  resizable: boolean;
  minWidth?: number;
  minHeight?: number;
  titleBarHeight?: number;
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

export const VnWindow = /*@__PURE__*/ defineComponent({
  name: "VnWindow",
  inheritAttrs: false,
  props: {
    /** The OS caption text. Live: changing it renames the window. */
    title: { type: String, default: "vue-native" },
    /** Where the window opens, CSS px. Creation-time only. */
    bounds: { type: Object as PropType<VnWindowBounds>, default: undefined },
    /** `"default"`: OS caption. `"hidden"`: the app paints its own `window-drag: move` region. */
    chrome: { type: String as PropType<"default" | "hidden">, default: "default" },
    /** `mica` | `mica-alt` | `acrylic` | `transparent` | `none`. Default: the app's own. */
    backdrop: { type: String, default: undefined },
    /** `HWND_TOPMOST`. Live. */
    alwaysOnTop: { type: Boolean, default: false },
    /** Creation-time only, as in gpui. */
    resizable: { type: Boolean, default: true },
    minWidth: { type: Number, default: undefined },
    minHeight: { type: Number, default: undefined },
    /** `env(titlebar-area-height)` inside this window, for `chrome: "hidden"`. */
    titleBarHeight: { type: Number, default: 32 },
    /** Take the keyboard on show? Default false: a palette that grabs focus steals the caret from the document. */
    activate: { type: Boolean, default: false },
    /** Live. `false` keeps the window created and painted but hidden, so the first `true` is a frame, not a creation. A hide returns activation to the main window. */
    visible: { type: Boolean, default: true },
    /** `"monitor"`: frameless, covers the whole monitor the main window is on (taskbar included); `bounds` ignored. Creation-time only. */
    cover: { type: String as PropType<"monitor">, default: undefined },
    /** `false`: owned by the main window: no taskbar button or Alt-Tab entry, stays above it, minimises with it. Creation-time only. */
    taskbar: { type: Boolean, default: true },
    /** Keep the full image decode while shown here (no reduced copy for small draws). Live. */
    fullResolution: { type: Boolean, default: false },
    /** The mouse passes THROUGH the whole window to whatever is under it (Electron `setIgnoreMouseEvents(true)`). Live. */
    ignoreMouseEvents: { type: Boolean, default: false },
    /** `false`: never activated, no taskbar or Alt-Tab entry, no keyboard (Electron `focusable: false`). Creation-time only. */
    focusable: { type: Boolean, default: true },
    /** Excluded from screen capture (`WDA_EXCLUDEFROMCAPTURE`). Live. */
    contentProtection: { type: Boolean, default: false },
    /** With `cover="monitor"`: monitor index in `EnumDisplayMonitors` order; absent means the main window's. Live: a shown window moves. */
    monitor: { type: Number as PropType<number | undefined>, default: undefined },
  },
  emits: {
    /** The OS window exists. Carries its gpui window id — what devtools `--window` names. */
    open: (_windowId: number) => true,
    close: () => true,
    focus: () => true,
    blur: () => true,
    resize: (_metrics: VnWindowMetrics) => true,
    show: (_shown: VnWindowShown) => true,
    hide: () => true,
  },
  setup(props, { slots, emit, expose, attrs }) {
    const el = ref<VnWindowTarget | null>(null);
    const windowId = ref(0);
    /** This pop-out's own viewport in CSS px. There is one JS realm, so `window.innerWidth`, `matchMedia` and `appWindow.*` keep meaning the primary window; CSS (`@media`, `env(titlebar-area-*)`) is evaluated against the window that paints the element. */
    const width = ref(0);
    const height = ref(0);
    const scale = ref(1);
    let closed = false;

    /** Where an element of the MAIN window is, in this window's CSS px relative to its root; null if either is unpainted or before open. */
    const mapFromMain = (target: VnWindowTarget | null | undefined): VnWindowRect | null => {
      if (closed || !el.value || !target || typeof __vn_window_map_rect !== "function") return null;
      const raw = __vn_window_map_rect(el.value.__vnId, target.__vnId);
      if (typeof raw !== "string") return null;
      const [x, y, w, h] = JSON.parse(raw) as number[];
      return { x: x!, y: y!, width: w!, height: h! };
    };

    expose({ el, windowId, width, height, scale, mapFromMain });

    const options = (): string => {
      const o: OpenOptions = {
        ...(props.bounds ?? {}),
        title: props.title,
        chrome: props.chrome,
        alwaysOnTop: props.alwaysOnTop,
        resizable: props.resizable,
        activate: props.activate,
        titleBarHeight: props.titleBarHeight,
        visible: props.visible,
        taskbar: props.taskbar,
        fullResolution: props.fullResolution,
        ignoreMouseEvents: props.ignoreMouseEvents,
        focusable: props.focusable,
        contentProtection: props.contentProtection,
      };
      if (props.monitor !== undefined) o.monitor = props.monitor;
      if (props.backdrop !== undefined) o.backdrop = props.backdrop;
      if (props.minWidth !== undefined) o.minWidth = props.minWidth;
      if (props.minHeight !== undefined) o.minHeight = props.minHeight;
      if (props.cover !== undefined) o.cover = props.cover;
      return JSON.stringify(o);
    };
    let sent = "";
    const sync = () => {
      if (closed || !el.value || !windowId.value) return;
      const now = options();
      if (now === sent) return;
      sent = now;
      __vn_window_set_options(el.value.__vnId, now);
    };

    const onEvent = (kind: string, detailJson: string) => {
      let detail: Record<string, number> = {};
      try {
        detail = JSON.parse(detailJson) as Record<string, number>;
      } catch {
        /* the host always sends an object; an unparsable one is not worth throwing over */
      }
      switch (kind) {
        case "open":
          windowId.value = detail.windowId ?? 0;
          sync();
          emit("open", windowId.value);
          break;
        case "show":
          width.value = detail.width || width.value;
          height.value = detail.height || height.value;
          scale.value = detail.scale || scale.value;
          emit("show", {
            firstFrameMs: detail.firstFrameMs ?? 0,
            width: width.value,
            height: height.value,
            scale: scale.value,
          });
          break;
        case "hide":
          emit("hide");
          break;
        case "focus":
          emit("focus");
          break;
        case "blur":
          emit("blur");
          break;
        case "resize":
          width.value = detail.width ?? 0;
          height.value = detail.height ?? 0;
          scale.value = detail.scale ?? 1;
          emit("resize", { width: width.value, height: height.value, scale: scale.value });
          break;
      }
    };

    onMounted(() => {
      const node = el.value;
      if (!node) return;
      // The host defers the open by one effect cycle (it nests in the primary window's update), so
      // no handle is returned; use `@open`. Un-painting is immediate: a node in two element trees
      // for one frame aliases gpui's scroll state and hitboxes.
      sent = options();
      __vn_window_open(
        node.__vnId,
        sent,
        () => {
          closed = true;
          windowId.value = 0;
          emit("close");
        },
        onEvent,
      );
    });

    // `onBeforeUnmount`, not `onUnmounted`: the node must exist when the close is requested; after
    // `onUnmounted` Vue has called `__vn_remove`.
    onBeforeUnmount(() => {
      if (closed || !el.value) return;
      __vn_window_close(el.value.__vnId);
    });
    // Leaves the registry after the content unmounts, so children's unmount hooks still see their window.
    let root: object | null = null;
    onMounted(() => (root = el.value));
    onUnmounted(() => {
      if (root) windowRoots.delete(root);
    });

    watch(
      () =>
        [
          props.title,
          props.bounds?.x,
          props.bounds?.y,
          props.bounds?.width,
          props.bounds?.height,
          props.alwaysOnTop,
          props.visible,
          props.fullResolution,
          props.ignoreMouseEvents,
          props.contentProtection,
          props.monitor,
          props.chrome,
          props.resizable,
          props.cover,
          props.taskbar,
          props.focusable,
        ] as const,
      sync,
    );

    // The pop-out's painted root: the host gives it the main window's root treatment (100%x100%, scroll, focus handle) so
    // content fills the window and an `<input>` can hold focus. It renders in this tree between mount and the deferred
    // open (`.vn-window-root`). Attrs are merged here because Vue's fall-through `for…in` would drop `WINDOW_ROOT`.
    return () => {
      const root: Record<string | symbol, unknown> = mergeProps({ ref: el, class: "vn-window-root" }, attrs);
      root[WINDOW_ROOT] = true;
      return h("div", root, slots.default?.());
    };
  },
});

export const VN_WINDOW_CREATION_TIME_PROPS: readonly string[] = CREATION_TIME_ONLY;
