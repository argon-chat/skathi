// `<SkWindow>` in Vue: a second OS window that paints part of THIS component tree. The subtree stays in this tree,
// component instance and reactive graph; only its paint moves, so nothing is remounted or serialized and a click in
// the pop-out runs the same handler.
//
//     <SkWindow v-if="open" title="Palette" :bounds="{ width: 320 }" @close="open = false">
//       <Palette v-model="brush" />
//     </SkWindow>
//
// A component rather than `<Teleport to="sk:window">`: like `window.open()` it supplies the target
// element, so `<Teleport :to="pal.el">` composes onto a `SkWindow` as onto a popup's body.

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
import {
  WINDOW_ROOT,
  createSkWindowPopout,
  windowRoots,
  type SkWindowBounds,
  type SkWindowMetrics,
  type SkWindowShown,
  type SkWindowState,
  type SkWindowTarget,
} from "./skwindow";

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
    /** Uses the window's own frame timer on macOS. Creation-time only. */
    lowLatencySurface: { type: Boolean, default: undefined },
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
