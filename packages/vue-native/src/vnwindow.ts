// `<VnWindow>` — a second OS window that paints part of THIS component tree.
//
// docs/MULTIWINDOW.md §1 (why this spelling), §3.3 (the contract), §5 (what the
// host had to make per-window for it to be true).
//
// The one-sentence model: the subtree inside `<VnWindow>` stays in this tree,
// this component instance and this reactive graph — only its PAINT moves to
// another window. Nothing is remounted, nothing is serialized, no store bridge
// is needed, and a click in the pop-out runs the same handler on the same
// `ref`:
//
//     <VnWindow v-if="open" title="Palette" :bounds="{ width: 320 }"
//               @close="open = false">
//       <Palette v-model="brush" />
//     </VnWindow>
//
// ## Why a component and not `<Teleport to="vn:window">`  (D-M02)
//
// In a browser, "teleport into another window" is not a special Teleport
// target: `window.open()` supplies an element and an ORDINARY
// `<Teleport :to="w.document.body">` moves DOM into it. Teleport does the
// moving; something else supplies the target. So vue-native supplies the
// target, and exposes it — `<Teleport :to="pal.el">` composes onto a `VnWindow`
// exactly the way it composes onto a popup's `document.body`:
//
//     <VnWindow ref="pal" title="Palette" />
//     <Teleport :to="pal.el"> <Palette /> </Teleport>
//
// A magic string in `to` would fail in a browser as Vue's "Failed to locate
// Teleport target" and render NOTHING — silence in exactly the direction the
// compat-first policy exists to prevent — and could not carry `title`,
// `bounds`, `chrome` or `backdrop` either.
//
// `window.open` is untouched and still reserved (A-22, docs/WINDOWING.md §8):
// it means "a second DOCUMENT", which is a different feature from "one tree,
// two windows".

import { defineComponent, h, onBeforeUnmount, onMounted, ref, watch } from "@vue/runtime-core";
import type { PropType } from "@vue/runtime-core";

/**
 * The container node `<VnWindow>` renders and the pop-out paints as its root.
 *
 * Structurally typed rather than imported: `runtime.ts`'s `MirrorElement` is
 * internal, and importing it here would make the module graph circular
 * (`runtime.ts` re-exports this file). `id` is the retained-tree node id, which
 * is the whole address the host needs (MULTIWINDOW.md §3.2 invariant 1).
 */
export interface VnWindowTarget {
  readonly id: number;
}

/** `<VnWindow :bounds>` — every field optional; absent means "let the OS pick". */
export interface VnWindowBounds {
  x?: number;
  y?: number;
  width?: number;
  height?: number;
}

/** The `resize` payload, in CSS px of the POP-OUT's own viewport. */
export interface VnWindowMetrics {
  width: number;
  height: number;
  scale: number;
}

/**
 * Props whose value gpui fixes when the window is created.
 *
 * Changing one of these on a live window **advises and does nothing** rather
 * than silently doing nothing — the same rule, for the same reason, as
 * `vn.config.json`'s `titleBarStyle` and `resizable` (docs/WINDOWING.md
 * D-25/D-30). Close and re-open the window (`v-if`) to change them.
 */
const CREATION_TIME_ONLY = ["chrome", "backdrop", "resizable", "minWidth", "minHeight"] as const;

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
}

export const VnWindow = /*@__PURE__*/ defineComponent({
  name: "VnWindow",
  props: {
    /** The OS caption text. Live: changing it renames the window. */
    title: { type: String, default: "vue-native" },
    /** Where the window opens, CSS px. Creation-time only. */
    bounds: { type: Object as PropType<VnWindowBounds>, default: undefined },
    /**
     * `"default"` — the OS draws a real caption: movable, resizable, closable,
     * Windows 11 snap layouts, no drag region needed.
     * `"hidden"` — the app owns the whole surface and must paint its own
     * `window-drag: move` region, or the window cannot be moved. Exactly the
     * contract `titleBarStyle: "hidden"` has for the main window.
     */
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
    /**
     * Does the window take the keyboard when it appears?
     *
     * **Default `false`**, and deliberately so: a tool palette that grabs focus
     * every time it opens is a bug in every desktop app — the user was typing
     * in the document and would lose the caret. Pass `activate` for a window
     * that is the user's new centre of attention (a detached editor, a
     * player).
     */
    activate: { type: Boolean, default: false },
  },
  emits: {
    /** The OS window exists. Carries its gpui window id — what devtools `--window` names. */
    open: (_windowId: number) => true,
    /** It closed, however it closed: the X button, `close()`, or the app quitting. */
    close: () => true,
    focus: () => true,
    blur: () => true,
    /** This window's own viewport moved. See the note on `width`/`height` below. */
    resize: (_metrics: VnWindowMetrics) => true,
  },
  setup(props, { slots, emit, expose }) {
    const el = ref<VnWindowTarget | null>(null);
    const windowId = ref(0);
    /**
     * **This pop-out's own viewport, in CSS px.**
     *
     * There is ONE JS realm for the whole process (D-M01), so there is one
     * `window` object: `window.innerWidth`, `matchMedia`, `document.title` and
     * every `appWindow.*` verb keep meaning the PRIMARY window and say so once
     * (A-M04, MULTIWINDOW.md §7.7). Inventing a second `window` that shadowed
     * the real one inside a subtree would be a divergence no browser has.
     *
     * So a pop-out's metrics are pushed to the component that asked for the
     * window instead — which is where an app can actually use them. CSS is the
     * other half and needs nothing: `@media (max-width: …)`,
     * `prefers-color-scheme`, `resolution` and `env(titlebar-area-*)` are
     * evaluated against the window that PAINTS the element (§6).
     */
    const width = ref(0);
    const height = ref(0);
    const scale = ref(1);
    /** `true` once the OS window is gone, so nothing tries to drive a dead id. */
    let closed = false;

    expose({ el, windowId, width, height, scale });

    const options = (): string => {
      const o: OpenOptions = {
        ...(props.bounds ?? {}),
        title: props.title,
        chrome: props.chrome,
        alwaysOnTop: props.alwaysOnTop,
        resizable: props.resizable,
        activate: props.activate,
        titleBarHeight: props.titleBarHeight,
      };
      if (props.backdrop !== undefined) o.backdrop = props.backdrop;
      if (props.minWidth !== undefined) o.minWidth = props.minWidth;
      if (props.minHeight !== undefined) o.minHeight = props.minHeight;
      return JSON.stringify(o);
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
          emit("open", windowId.value);
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
      // The host DEFERS the open by one effect cycle: `App::open_window` draws
      // the new window once before returning, and this call is nested inside
      // the primary window's own update. So there is no handle to return here
      // and none is promised — `@open` is the honest shape (§3.2 invariant 2).
      // Un-painting, though, is immediate: the main window's very next draw is
      // already without this subtree, because a node in two element trees for
      // one frame aliases gpui's scroll state and hitboxes (§3.2 invariant 3).
      __vn_window_open(node.id, options(), () => {
        closed = true;
        windowId.value = 0;
        emit("close");
      }, onEvent);
    });

    // `onBeforeUnmount`, NOT `onUnmounted`: the node must still exist when the
    // close is requested. After `onUnmounted` Vue has already called
    // `__vn_remove` and the host would refuse an id that is gone (§3.3).
    onBeforeUnmount(() => {
      if (closed || !el.value) return;
      __vn_window_close(el.value.id);
    });

    // The LIVE props. `title` renames the window; `alwaysOnTop` re-stacks it.
    // Everything else is creation-time-only in gpui and the host advises rather
    // than pretending — see CREATION_TIME_ONLY.
    watch(
      () => [props.title, props.alwaysOnTop, props.chrome, props.resizable] as const,
      () => {
        if (closed || !el.value) return;
        __vn_window_set_options(el.value.id, options());
      },
    );

    // The container is an ordinary element node and is the pop-out's PAINTED
    // ROOT, so the host gives it the root treatment `Tree::root` gets in the
    // main window — 100 %×100 %, `overflow: scroll`, a tracked focus handle
    // (MULTIWINDOW.md §7.1). That is what makes the content fill the window and
    // what lets an `<input>` inside it hold keyboard focus.
    //
    // It renders in THIS window's tree for exactly the frames between mount and
    // the deferred open; `class="vn-window-root"` is the handle an app has if
    // it wants to style those frames.
    return () => h("div", { ref: el, class: "vn-window-root" }, slots.default?.());
  },
});

/** The names an app can change on a live `<VnWindow>` — the rest re-open it. */
export const VN_WINDOW_CREATION_TIME_PROPS: readonly string[] = CREATION_TIME_ONLY;
