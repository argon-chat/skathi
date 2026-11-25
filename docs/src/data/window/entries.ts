import type { Platform, PlatformSince, Release, Status, ValueSupport } from "../css/entries";
import type { Detail } from "../details";

export type Kind = "method" | "property" | "event" | "component" | "prop" | "exposed" | "option" | "function";

export const categories = [
  "Main window",
  "Window events",
  "Startup configuration",
  "Title bar and chrome",
  "Pop-out windows",
  "Taskbar and dock",
] as const;

export type Category = (typeof categories)[number];

export const categoryNotes: Readonly<Partial<Record<Category, string>>> = {
  "Main window":
    "`appWindow` and the window globals (`innerWidth`, `screen`, `document.title`, the Fullscreen API) always address the primary window, also when the code runs from a pop-out's event handler. Every number is in CSS px. Verbs return at once: inside an input handler they apply in the same frame, otherwise at the top of the next one. On Linux under Wayland, the compositor does not let an app place, hide or pin its window; each entry lists what it does there instead. See [Windows](/guides/windows/).",
  "Window events":
    "Most host events are plain `Event`s. The transition events `fullscreenwillchange` and `fullscreencancel` include the destination in `detail.fullscreen`. For the other events, read the new state from `appWindow` in the handler, where the getters are already current. In one frame they fire in a fixed order: `resize`, `visibilitychange`, then `maximize` or `unmaximize`, `minimize` or `restore`, `fullscreenchange`, `backdropchange`, `focus` or `blur`, `move`, `geometrychange`, and last the media query `change` events. Verbs called inside a handler apply on the next frame. See [Closing and running in the background](/guides/closing-and-background/).",
  "Startup configuration":
    "The `window` block of `sk.config.json` is read once, before the main window opens, and a change needs a restart. It is the only place to set the size, the minimum size, `titleBarStyle`, `resizable`, `minimizable` and `maximizable`. A key the host does not know is ignored without a warning. `defineWindow()` applies the keys that can change at run time, after the window exists.",
  "Title bar and chrome":
    'With the default `titleBarStyle: "hidden"`, the app draws its own title bar under the Window Controls Overlay model: `app-region` marks where the window drags, `--window-control` turns elements into caption buttons, and `env(titlebar-area-*)` and `navigator.windowControlsOverlay` describe the strip. On macOS, the traffic lights stay at the left of the strip, and the title bar area starts past them. See [Custom title bar](/guides/custom-title-bar/).',
  "Pop-out windows":
    "`<SkWindow>` paints part of the same component tree in a second OS window: one JS realm, one Vue or Svelte app, the same reactive state. Mounting the component opens the window and unmounting it closes it. CSS inside it is evaluated against the pop-out's own viewport, while `window.*` and `appWindow` still mean the main window. `taskbar`, `focusable` and `mapFromMain()` work on Windows and macOS, and `monitor` only on Windows. A Svelte app imports it from `@skathi-native/runtime/svelte`, where the events are callback props such as `onclose` and the exposed values are `bind:` props. See [Pop-out windows](/guides/pop-out-windows/).",
  "Taskbar and dock":
    "The badge on the app icon is `navigator.setAppBadge()`. `appWindow` has no taskbar progress, overlay icon or skip-taskbar call; on Windows, `appWindow.hide()` removes the taskbar button with the window. The window and taskbar icon comes from `sk.config.json` at build time and cannot change at run time.",
};

export interface WindowEntry extends Detail {
  slug: string;
  name: string;
  kind: Kind;
  category: Category;
  summary: string;
  status: Status;
  platforms: Record<Platform, Status>;
  since?: Release;
  platformSince?: PlatformSince;
  skathiOnly?: boolean;
  mdn?: string;
  values?: ValueSupport[];
  notes?: string[];
  limitations?: string[];
  workaround?: string;
  related?: string[];
  aliases?: string[];
}

const OK: Status = "supported";
const PART: Status = "partial";
const PLAN: Status = "planned";
const NO: Status = "not-planned";

const FIRST_RELEASE: Release = "0.2.0";
const MDN = "https://developer.mozilla.org/docs/Web/";

const all = (status: Status): Record<Platform, Status> => ({ windows: status, macos: status, linux: status });
const os = (windows: Status, macos: Status, linux: Status): Record<Platform, Status> => ({ windows, macos, linux });

const MAC_LINUX_0_3: PlatformSince = { macos: "0.3.0", linux: "0.3.0" };

function v(value: string, status: Status, note?: string): ValueSupport {
  return { value, status, ...(note === undefined ? {} : { note }) };
}

function overall(p: Record<Platform, Status>): Status {
  const s = [p.windows, p.macos, p.linux];
  if (s.every((x) => x === p.windows)) return p.windows;
  if (s.some((x) => x === OK || x === PART)) return PART;
  return s.includes(PLAN) ? PLAN : NO;
}

type Draft = Omit<WindowEntry, "category" | "status" | "skathiOnly"> & { web?: boolean };

function section(category: Category, drafts: readonly Draft[]): WindowEntry[] {
  return drafts.map(({ web, ...d }) => {
    const status = overall(d.platforms);
    const e: WindowEntry = { ...d, category, status };
    if (status === OK || status === PART) e.since = d.since ?? FIRST_RELEASE;
    if (!web && d.mdn === undefined) e.skathiOnly = true;
    return e;
  });
}

const IMPORT = 'import { appWindow } from "skathi:app";';
const WAYLAND_NO_POSITION =
  "On Linux under Wayland, the compositor places the window: a position is ignored with an advisory, and a size still applies.";
const MAC_LINUX_SAME_AS_CONTENT =
  "On macOS and Linux, it equals the content bounds. On macOS, `x` and `y` are relative to the screen the window is on, and the frame includes the title bar.";

const mainWindow = section("Main window", [
  {
    slug: "set-min-size",
    name: "appWindow.setMinSize()",
    kind: "method",
    summary: "Sets the minimum client size in CSS pixels while the main window is open.",
    platforms: all(OK),
    since: "0.6.0",
    syntax: "appWindow.setMinSize({ width, height } | null): void",
    notes: [
      "A larger minimum immediately grows a windowed client. Use zero for an unconstrained axis. The same limit applies to dragging and programmatic bounds. Passing null clears the startup or runtime minimum.",
      "Windows 11 snap layouts can reject windows whose minimum width exceeds 500 effective pixels.",
    ],
  },
  {
    slug: "set-max-size",
    name: "appWindow.setMaxSize()",
    kind: "method",
    summary: "Sets the maximum client size in CSS pixels while the main window is open.",
    platforms: all(OK),
    since: "0.6.0",
    syntax: "appWindow.setMaxSize({ width, height } | null): void",
    notes: [
      "A smaller maximum shrinks a windowed client. Pass null to clear it. The minimum takes priority when two limits conflict.",
    ],
  },
  {
    slug: "set-aspect-ratio",
    name: "appWindow.setAspectRatio()",
    kind: "method",
    summary: "Locks the width-to-height ratio of the native client area.",
    platforms: all(OK),
    since: "0.6.0",
    syntax: "appWindow.setAspectRatio(ratio: number | null): void",
    notes: [
      "Use 16 / 9 for a widescreen player. A width in setBounds or setContentBounds drives the height. A height-only request drives the width. The native window frame and caption are accounted for separately.",
      "The constraint applies during live resize on Windows and macOS, through geometry hints on X11 and during interactive xdg-shell resize on Wayland. Fullscreen and maximization use the compositor’s dimensions. Pass null to release the lock.",
    ],
  },
  {
    slug: "set-window-buttons-visible",
    name: "appWindow.setWindowButtonsVisible()",
    kind: "method",
    summary: "Shows or hides the main window’s caption buttons.",
    platforms: os(PART, OK, PART),
    since: "0.6.0",
    syntax: "appWindow.setWindowButtonsVisible(visible: boolean, options?: { fade?: number }): void",
    notes: [
      "On macOS this controls the three AppKit traffic lights. Showing them restores their position for the current title-bar height or the coordinates set by setTrafficLightPosition(). Skathi updates their tracking areas and cursor rects after placement.",
      "Pass a nonnegative fade duration in milliseconds to match a CSS opacity transition. The buttons stay interactive during a fade out and disappear from hit testing and geometry when it finishes. Reversing a fade starts at the current opacity. Reduced motion changes visibility immediately.",
      "On Windows and Linux this controls the host-drawn cluster enabled by titleBarOverlay. The desktop controls its own standard title bar. The visibility preference survives a change of title-bar height or a round trip through fullscreen.",
    ],
    related: ["get-window-buttons-rect", "set-title-bar-overlay"],
  },
  {
    slug: "get-window-buttons-rect",
    name: "appWindow.getWindowButtonsRect()",
    kind: "method",
    summary: "Reads the bounds of the visible caption buttons in the window.",
    platforms: os(PART, OK, PART),
    since: "0.6.0",
    syntax: "appWindow.getWindowButtonsRect(): { x: number; y: number; width: number; height: number } | null",
    notes: [
      "Coordinates are CSS pixels from the top left of the native content area. The rectangle encloses the whole cluster, including the spaces between buttons. The getter returns null while the cluster is hidden or unavailable. A standard macOS title bar can have a negative y relative to the content.",
      "For a custom title bar covering the viewport, compare pointer clientX and clientY against this rectangle before hiding the cursor or starting an idle animation. Read it again after geometrychange.",
    ],
    related: ["set-window-buttons-visible", "set-traffic-light-position"],
  },
  {
    slug: "set-traffic-light-position",
    name: "appWindow.setTrafficLightPosition()",
    kind: "method",
    summary: "Places the macOS caption buttons in the title bar.",
    platforms: os(NO, OK, NO),
    since: "0.6.0",
    syntax: "appWindow.setTrafficLightPosition(position: { x: number; y: number } | null): void",
    notes: [
      "The coordinates are the close button’s origin in its AppKit title-bar container, in points from the left and bottom. The other two buttons follow at their native spacing.",
      "In a custom title bar, null restores automatic centering from its current height. A standard title bar restores the AppKit positions. The setting survives native fullscreen. Read the windowed position from appWindow.chrome.trafficLightPosition.",
    ],
  },
  {
    slug: "opacity",
    name: "appWindow.opacity",
    kind: "property",
    summary: "Sets the opacity of the entire native window.",
    platforms: os(OK, OK, PART),
    since: "0.6.0",
    syntax: "appWindow.opacity: number",
    notes: [
      "Use 0 for transparent and 1 for opaque. A fade restores this target opacity when the window is shown again. Direct writes cancel an active fade.",
      "Windows uses layered window alpha, macOS uses AppKit alphaValue and X11 requests compositor opacity. Wayland uses the compositor’s normal opacity.",
    ],
  },
  {
    slug: "visible-on-all-workspaces",
    name: "appWindow.visibleOnAllWorkspaces",
    kind: "property",
    summary: "Requests that the window join every workspace.",
    platforms: os(NO, OK, PART),
    since: "0.6.0",
    syntax: "appWindow.visibleOnAllWorkspaces: boolean",
    notes: [
      "When both alwaysOnTop and visibleOnAllWorkspaces are true, macOS hosts the existing content view in a nonactivating NSPanel. It floats on other apps’ fullscreen Spaces, can receive keyboard input and leaves the active app’s menu in place. Clearing either flag returns the content to its original window.",
      "Windows topmost applies to the current virtual desktop. On X11 the window manager handles the all-desktops request. Wayland delegates workspace membership to the compositor.",
    ],
  },
  {
    slug: "app-window",
    name: "appWindow",
    kind: "property",
    summary: "The object that reads and controls the app's main window: its state, its size, its title and its events.",
    platforms: os(OK, OK, PART),
    syntax: `${IMPORT}\n\nconst appWindow: SkAppWindow; // extends EventTarget`,
    notes: [
      "It is imported from `skathi:app`. Importing it from `@skathi-native/runtime` still works but is deprecated.",
      "Its getters and its `is*()` and `get*()` methods are reactive: read inside a template, `computed` or `watch`, they update when the window changes. In Svelte, they are reactive in markup, `$derived` and `$effect` once the app imports `@skathi-native/runtime/svelte`.",
      "Every verb is synchronous and returns `undefined`. Inside an input handler it applies in the same frame; elsewhere, at the top of the next frame.",
      "It always addresses the main window, also when the code runs from a pop-out's handler. A pop-out has no handle object; it is controlled through the props of `<SkWindow>`.",
      "It is an `EventTarget` for the host events: `close-requested`, `resize`, `focus`, `blur`, `maximize`, `unmaximize`, `minimize`, `restore`, `move`, `fullscreenchange` and `backdropchange`. `resize`, `focus` and `blur` also fire on the global `window`.",
      "Like Tauri's `getCurrentWindow()` or Electron's `BrowserWindow`, with the verbs named the same way.",
    ],
    examples: [
      {
        title: "A status bar that follows the window state",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}
</script>

<template>
  <footer class="status-bar">
    <span>{{ appWindow.isMaximized() ? "Maximized" : "Windowed" }}</span>
    <span>{{ Math.round(appWindow.getBounds().width) }} px wide</span>
    <span v-if="!appWindow.isFocused()">Inactive</span>
  </footer>
</template>`,
      },
    ],
    edgeCases: [
      "There are no `on<event>` handler properties such as `appWindow.onmaximize`. Use `addEventListener`.",
      "In a unit test without the host, `appWindow` is an inert stand-in: `is*()` methods return `false`, every other key reads as a no-op function, writes are dropped, and each access warns once.",
      "`event.target === appWindow` holds inside a listener.",
    ],
    limitations: [
      "On Linux under Wayland, the verbs that place, hide or pin the window fall back or do nothing; each entry lists the gap.",
    ],
    related: ["get-current-window", "event-close-requested", "chrome", "define-window"],
    aliases: ["BrowserWindow", "window handle", "getCurrentWindow"],
  },
  {
    slug: "get-current-window",
    name: "getCurrentWindow()",
    kind: "function",
    summary: "Returns `appWindow`, so code written for Tauri's `getCurrentWindow()` runs unchanged.",
    platforms: all(OK),
    syntax: 'import { getCurrentWindow } from "@skathi-native/runtime";\n\ngetCurrentWindow(): SkAppWindow',
    returns: "`appWindow`, the main window.",
    notes: ["It returns the main window also when called from code inside a pop-out."],
    examples: [
      {
        title: "Port a Tauri title bar helper",
        lang: "ts",
        code: `import { getCurrentWindow } from "@skathi-native/runtime";

export function wireTitlebar(min: HTMLElement, max: HTMLElement, close: HTMLElement) {
  const win = getCurrentWindow();
  min.addEventListener("click", () => win.minimize());
  max.addEventListener("click", () => win.toggleMaximize());
  close.addEventListener("click", () => win.close());
}`,
        note: "`--window-control` on the buttons gives the native behavior instead, including the snap layouts flyout on Windows.",
      },
    ],
    related: ["app-window", "sk-window"],
    aliases: ["Tauri"],
  },
  {
    slug: "minimize",
    name: "appWindow.minimize()",
    kind: "method",
    summary: "Minimizes the main window to the taskbar or the Dock.",
    platforms: os(OK, OK, PART),
    syntax: "appWindow.minimize(): void",
    returns: "`undefined`.",
    notes: [
      "`visibilitychange` (to hidden) and then `minimize` fire in the same frame. Restoring fires `visibilitychange` (to visible) and then `restore`.",
      "Calling it on a minimized window does nothing.",
      "It works when `window.minimizable` is `false` in `sk.config.json`.",
    ],
    examples: [
      {
        title: "Get out of the way while a long export runs",
        lang: "ts",
        code: `${IMPORT}

export async function exportAndMinimize(run: () => Promise<void>) {
  appWindow.minimize();
  await run();
  appWindow.restore();
}`,
      },
    ],
    limitations: [
      "On Linux under Wayland, the compositor may refuse it. When it minimizes, `isMinimized()` stays `false` and no `minimize`, `restore` or `visibilitychange` event fires.",
    ],
    related: ["is-minimized", "restore", "event-minimize", "event-restore"],
  },
  {
    slug: "maximize",
    name: "appWindow.maximize()",
    kind: "method",
    summary: "Maximizes the main window to fill the work area of its monitor.",
    platforms: all(OK),
    syntax: "appWindow.maximize(): void",
    returns: "`undefined`.",
    notes: [
      "Calling it on a maximized window does nothing. The `maximize` event and a `geometrychange` follow.",
      "On macOS, it zooms the window, and maximized means zoomed.",
      "On Windows, a call before the window is first shown, such as at the top of the app's entry module, makes the first show maximized.",
      "A maximized window that enters and leaves fullscreen comes back maximized.",
    ],
    examples: [
      {
        title: "Open maximized on the first launch",
        lang: "ts",
        code: `${IMPORT}

if (!localStorage.getItem("launched")) {
  appWindow.maximize();
  localStorage.setItem("launched", "1");
}`,
        note: "Run it at the top of the entry module, before the first frame, so the window appears maximized.",
      },
    ],
    related: ["unmaximize", "toggle-maximize", "is-maximized", "event-maximize"],
    aliases: ["zoom"],
  },
  {
    slug: "unmaximize",
    name: "appWindow.unmaximize()",
    kind: "method",
    summary: "Restores a maximized main window to its previous size and position.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.unmaximize(): void",
    returns: "`undefined`.",
    notes: [
      "On a minimized window, it restores the window from minimized to its earlier state, which may be maximized.",
      "The `unmaximize` event follows.",
    ],
    examples: [
      {
        title: "Leave maximized before snapping to a saved size",
        lang: "ts",
        code: `${IMPORT}

export function applyLayout(rect: { x: number; y: number; width: number; height: number }) {
  if (appWindow.isMaximized()) appWindow.unmaximize();
  appWindow.setBounds(rect);
}`,
      },
    ],
    limitations: ["On Linux under Wayland, a minimized window is not known as minimized, so it is not restored."],
    related: ["maximize", "restore", "toggle-maximize", "event-unmaximize"],
  },
  {
    slug: "toggle-maximize",
    name: "appWindow.toggleMaximize()",
    kind: "method",
    summary: "Maximizes the main window, or restores it if it is maximized.",
    platforms: all(OK),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.toggleMaximize(): void",
    returns: "`undefined`.",
    examples: [
      {
        title: "Maximize from a double-click on a toolbar",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}
</script>

<template>
  <nav class="toolbar" @dblclick.self="appWindow.toggleMaximize()">
    <slot />
  </nav>
</template>`,
        note: "An `app-region: drag` area already maximizes on a double-click; use this for areas that do not drag.",
      },
    ],
    related: ["maximize", "unmaximize", "is-maximized"],
  },
  {
    slug: "restore",
    name: "appWindow.restore()",
    kind: "method",
    summary: "Brings a minimized or maximized main window back to its normal size.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.restore(): void",
    returns: "`undefined`.",
    notes: [
      "The same call as `unmaximize()`: it leaves either state.",
      "It does not show a window hidden with `hide()`; call `show()`.",
      "The `restore` event means only that the window left the minimized state.",
    ],
    examples: [
      {
        title: "Come back when a download finishes",
        lang: "ts",
        code: `${IMPORT}

export function onDownloadDone(file: string) {
  if (appWindow.isMinimized()) appWindow.restore();
  appWindow.title = "Downloaded " + file;
}`,
      },
    ],
    limitations: ["On Linux under Wayland, a minimized window is not known as minimized, so it is not restored."],
    related: ["unmaximize", "minimize", "show", "event-restore"],
  },
  {
    slug: "close",
    name: "appWindow.close()",
    kind: "method",
    summary: "Closes the main window and quits the app, without asking `close-requested`.",
    platforms: all(OK),
    syntax: "appWindow.close(): void",
    returns: "`undefined`.",
    notes: [
      "It dispatches `pagehide` on `window` at once, then hides every window of the app, closes the pop-outs and quits.",
      "`close-requested` and `beforeunload` are not dispatched. Call it after the app has asked the user in its own `close-requested` handler.",
      "A second call does nothing.",
      "The same function as `window.close()`.",
    ],
    examples: [
      {
        title: "Ask before closing with unsaved changes",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, ref } from "vue";
${IMPORT}

const props = defineProps<{ dirty: boolean }>();
const asking = ref(false);

onMounted(() =>
  appWindow.addEventListener("close-requested", (e) => {
    if (!props.dirty) return;
    e.preventDefault();
    asking.value = true;
  }),
);
</script>

<template>
  <div v-if="asking" class="dialog" role="dialog">
    <p>Discard your unsaved changes?</p>
    <button @click="appWindow.close()">Discard</button>
    <button @click="asking = false">Keep editing</button>
  </div>
</template>`,
      },
    ],
    edgeCases: [
      "It closes the app, not only the window: there is no way to close the main window and keep a pop-out open.",
      "A file dialog that the window owns closes first, which delays the close by about 100 ms.",
    ],
    related: ["event-close-requested", "event-pagehide", "window-close", "hide"],
    aliases: ["quit", "exit"],
  },
  {
    slug: "center",
    name: "appWindow.center()",
    kind: "method",
    summary: "Moves the main window to the center of the work area of its monitor.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.center(): void",
    returns: "`undefined`.",
    notes: ["The outer rectangle, frame included, is centered within the work area of the monitor the window is on."],
    limitations: ["On Linux under Wayland, it does nothing and prints an advisory: the compositor places the window."],
    related: ["set-bounds", "screen", "get-bounds"],
  },
  {
    slug: "show",
    name: "appWindow.show()",
    kind: "method",
    summary: "Shows the main window and brings it to the front, also after `hide()`.",
    platforms: os(OK, OK, PART),
    syntax: "appWindow.show(options?: { fade?: number }): void",
    returns: "`undefined`.",
    notes: [
      "Pass { fade: 180 } to show a hidden window at zero alpha and fade to its configured opacity over 180 ms. A new fade can reverse an active one.",
      "On a hidden window it shows the window, then activates it. On a shown window it activates it.",
      "It always raises the window, whatever `window.focus` says in `sk.config.json`. On macOS, it brings the app to the front as well.",
      "It cancels the 10 s fallback that shows a window started hidden with no tray icon.",
    ],
    examples: [
      {
        title: "Open the window from the tray icon",
        lang: "ts",
        code: `import { appTray, appWindow } from "skathi:app";

if (appTray.available) {
  appTray.set({
    icon: "assets/tray.png",
    tooltip: "Inbox",
    onClick: () => appWindow.show(),
    menu: [{ label: "Open Inbox", onSelect: () => appWindow.show() }, { type: "separator" }, { role: "quit" }],
  });
}`,
      },
    ],
    limitations: [
      "On Linux under Wayland, `hide()` minimizes the window, and the compositor may only flag the window for attention instead of raising it.",
    ],
    related: ["hide", "app-tray", "window-focus", "is-visible"],
  },
  {
    slug: "hide",
    name: "appWindow.hide()",
    kind: "method",
    summary: "Hides the main window and its taskbar button while the app keeps running.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.hide(options?: { fade?: number }): void",
    returns: "`undefined`.",
    notes: [
      "`document.hidden` becomes `true`, `isVisible()` returns `false` and `visibilitychange` fires. No `minimize` event fires.",
      "Pass { fade: 180 } to fade out over 180 ms before hiding. Wayland falls back to its ordinary hide behavior.",
      "Call `show()` to bring the window back.",
      "Like Electron's `win.hide()`.",
    ],
    examples: [
      {
        title: "Hide to the tray instead of closing",
        lang: "ts",
        code: `import { appTray, appWindow } from "skathi:app";

appWindow.addEventListener("close-requested", (e) => {
  if (!appTray.available) return;
  e.preventDefault();
  appWindow.hide();
});`,
        note: 'Pair it with a tray icon whose click calls `appWindow.show()`, and a `role: "quit"` menu item to quit.',
      },
    ],
    limitations: [
      "On Linux under Wayland, it minimizes the window instead. The window stays in the taskbar, `isVisible()` stays `true` and no event fires.",
    ],
    related: ["show", "event-close-requested", "app-tray", "is-visible", "document-visibility-state"],
    aliases: ["hide to tray", "minimize to tray"],
  },
  {
    slug: "is-visible",
    name: "appWindow.isVisible()",
    kind: "method",
    summary: "Returns whether the main window is shown, as opposed to hidden with `hide()`.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.isVisible(): boolean",
    returns: "`true` when the window is shown. A minimized window counts as shown.",
    notes: ["It re-reads the window first, so it is exact right after a verb in the same handler.", "It is reactive."],
    examples: [
      {
        title: "Play a sound only while the window is hidden",
        lang: "ts",
        code: `${IMPORT}

export function onMessage(chime: HTMLAudioElement) {
  if (!appWindow.isVisible()) void chime.play();
}`,
      },
    ],
    edgeCases: ["On Windows, it returns `true` while the window waits for its first frame before being shown."],
    limitations: ["On Linux under Wayland, it always returns `true`."],
    related: ["hide", "show", "document-visibility-state", "is-minimized"],
  },
  {
    slug: "set-fullscreen",
    name: "appWindow.setFullscreen()",
    kind: "method",
    summary: "Puts the main window in fullscreen or takes it out.",
    platforms: all(OK),
    syntax: 'appWindow.setFullscreen(on: boolean, options?: { transition?: "system" | "fade" }): void',
    params: [{ name: "on", type: "boolean", description: "`true` enters fullscreen, `false` leaves it." }],
    returns: "`undefined`.",
    notes: [
      "Asking for the current state does nothing.",
      "The default transition is system. With transition set to fade, macOS covers the content in black before AppKit captures it and reveals it over 150 ms after completion or cancellation. Window opacity stays at its configured value. Reduced motion, Windows and Linux use the system transition.",
      "On Windows, fullscreen covers the monitor the window is on, and leaving restores the exact windowed or maximized rectangle.",
      "In fullscreen, `display-mode` is `fullscreen`, `env(titlebar-area-*)` is undefined, and the caption buttons from `titleBarOverlay` are hidden.",
      "It does not set `document.fullscreenElement`, and Escape does not leave it. Use `element.requestFullscreen()` for that behavior.",
    ],
    examples: [
      {
        title: "Toggle presentation mode with F11",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, onUnmounted } from "vue";
${IMPORT}

function onKey(e: KeyboardEvent) {
  if (e.key !== "F11") return;
  e.preventDefault();
  appWindow.setFullscreen(!appWindow.isFullscreen());
}

onMounted(() => addEventListener("keydown", onKey));
onUnmounted(() => removeEventListener("keydown", onKey));
</script>

<template>
  <slot />
</template>`,
      },
    ],
    edgeCases: ["While the window is hidden, the call is kept and the window shows in fullscreen."],
    limitations: [
      "On macOS, it is native fullscreen in its own Space, with the animation. The backdrop is not drawn there, so `effectiveBackdrop` reads `none`.",
      "On Linux, the window manager decides whether to grant it.",
    ],
    related: ["is-fullscreen", "request-fullscreen", "event-fullscreenchange", "display-mode"],
  },
  {
    slug: "is-maximized",
    name: "appWindow.isMaximized()",
    kind: "method",
    summary: "Returns whether the main window is maximized.",
    platforms: all(OK),
    syntax: "appWindow.isMaximized(): boolean",
    returns: "`true` while the window is maximized.",
    notes: [
      "It is reactive and re-reads the window first, so it is exact right after `maximize()`.",
      "On macOS, it is `true` while the window is zoomed.",
    ],
    examples: [
      {
        title: "Swap the maximize glyph in a custom title bar",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}
</script>

<template>
  <button class="max" :title="appWindow.isMaximized() ? 'Restore' : 'Maximize'">
    {{ appWindow.isMaximized() ? "❐" : "□" }}
  </button>
</template>

<style scoped>
.max {
  --window-control: maximize;
}
</style>`,
      },
    ],
    related: ["maximize", "toggle-maximize", "event-maximize", "event-unmaximize"],
  },
  {
    slug: "is-minimized",
    name: "appWindow.isMinimized()",
    kind: "method",
    summary: "Returns whether the main window is minimized.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.isMinimized(): boolean",
    returns: "`true` while the window is minimized.",
    notes: ["It is reactive and re-reads the window first."],
    examples: [
      {
        title: "Count unread messages while minimized",
        lang: "ts",
        code: `${IMPORT}

let unread = 0;

export function onMessage() {
  if (!appWindow.isMinimized()) return;
  unread += 1;
  void navigator.setAppBadge(unread);
}

appWindow.addEventListener("restore", () => {
  unread = 0;
  void navigator.clearAppBadge();
});`,
      },
    ],
    limitations: ["On Linux under Wayland, it always returns `false`."],
    related: ["minimize", "restore", "event-minimize", "document-visibility-state"],
  },
  {
    slug: "is-fullscreen",
    name: "appWindow.isFullscreen()",
    kind: "method",
    summary: "Returns whether the main window is fullscreen, by any route.",
    platforms: all(OK),
    syntax: "appWindow.isFullscreen(): boolean",
    returns: "`true` while the window is fullscreen.",
    notes: [
      "It covers `setFullscreen()`, element fullscreen in the main window and the macOS green button.",
      "It is reactive.",
    ],
    examples: [
      {
        title: "Hide the title bar in fullscreen",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}
</script>

<template>
  <header v-if="!appWindow.isFullscreen()" class="titlebar">Editor</header>
  <main class="content"><slot /></main>
</template>`,
        note: "`@media (display-mode: fullscreen)` does the same from CSS.",
      },
    ],
    related: ["set-fullscreen", "event-fullscreenchange", "fullscreen-element"],
  },
  {
    slug: "is-focused",
    name: "appWindow.isFocused()",
    kind: "method",
    summary: "Returns whether the main window is the active window.",
    platforms: all(OK),
    syntax: "appWindow.isFocused(): boolean",
    returns: "`true` while the main window is active.",
    notes: ["It is `false` while one of the app's own pop-outs is active.", "It is reactive."],
    examples: [
      {
        title: "Dim the title bar while the window is inactive",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}
</script>

<template>
  <header class="titlebar" :class="{ inactive: !appWindow.isFocused() }">Notes</header>
</template>

<style scoped>
.titlebar.inactive {
  opacity: 0.6;
}
</style>`,
      },
    ],
    related: ["event-focus", "event-blur", "document-has-focus", "sk-window-event-focus"],
  },
  {
    slug: "get-bounds",
    name: "appWindow.getBounds()",
    kind: "method",
    summary: "Returns the position and size of the main window, frame included, in CSS px.",
    platforms: os(OK, PART, PART),
    syntax: "appWindow.getBounds(): SkWindowRect // { x, y, width, height }",
    returns: "A new `{ x, y, width, height }` object on each call.",
    notes: [
      "On Windows, it is the outer rectangle of the window divided by the window's display scale.",
      "It reads the state published at the last frame, so right after `setBounds()` or `maximize()` it can return the previous geometry until the next frame.",
      "It is reactive.",
    ],
    examples: [
      {
        title: "Remember the window size between launches",
        lang: "ts",
        code: `${IMPORT}

const saved = localStorage.getItem("window-bounds");
if (saved) appWindow.setBounds(JSON.parse(saved));

addEventListener("pagehide", () => {
  if (appWindow.isMaximized() || appWindow.isMinimized()) return;
  localStorage.setItem("window-bounds", JSON.stringify(appWindow.getBounds()));
});`,
        note: "Writes to `localStorage` in `pagehide` reach disk. On Linux under Wayland, the position is not restored.",
      },
    ],
    edgeCases: [
      "On Windows, a minimized window reads a position near `-32000`, so do not save it while minimized.",
      "With monitors at different scales, the position on another monitor is divided by the current monitor's scale.",
    ],
    limitations: [MAC_LINUX_SAME_AS_CONTENT],
    related: ["set-bounds", "get-content-bounds", "screen-x", "outer-width"],
  },
  {
    slug: "get-content-bounds",
    name: "appWindow.getContentBounds()",
    kind: "method",
    summary: "Returns the position and size of the main window's client area in CSS px.",
    platforms: os(OK, PART, PART),
    syntax: "appWindow.getContentBounds(): SkWindowRect // { x, y, width, height }",
    returns: "A new `{ x, y, width, height }` object on each call.",
    notes: [
      "On Windows, it is the client area: the window without its frame.",
      "Like `getBounds()`, it reads the state of the last frame. It is reactive.",
    ],
    examples: [
      {
        title: "Attach the window size to a bug report",
        lang: "ts",
        code: `${IMPORT}

export function windowInfo() {
  const { width, height } = appWindow.getContentBounds();
  return {
    size: Math.round(width) + "x" + Math.round(height),
    scale: appWindow.scaleFactor,
    maximized: appWindow.isMaximized(),
  };
}`,
      },
    ],
    limitations: [
      "On macOS, it is the whole window frame, title bar included, with `x` and `y` relative to the screen the window is on.",
      "On Linux under Wayland, the position is not reported reliably.",
    ],
    related: ["get-bounds", "set-content-bounds", "inner-width"],
  },
  {
    slug: "set-bounds",
    name: "appWindow.setBounds()",
    kind: "method",
    summary: "Moves or resizes the main window, frame included, in CSS px.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax:
      'appWindow.setBounds(rect: Partial<SkWindowRect>, options?: { animate?: boolean | { duration?: number; easing?: "linear" | "ease-in" | "ease-out" | "ease-in-out" } }): void',
    params: [
      { name: "rect.x", type: "number", description: "Left edge of the outer window.", optional: true },
      { name: "rect.y", type: "number", description: "Top edge of the outer window.", optional: true },
      { name: "rect.width", type: "number", description: "Outer width.", optional: true },
      { name: "rect.height", type: "number", description: "Outer height.", optional: true },
    ],
    returns: "`undefined`. A call the OS refuses changes nothing, without an error.",
    notes: [
      "A field that is missing or not a finite number keeps its current value, so `{ x, y }` moves without resizing.",
      "The window is neither raised nor activated.",
      "The size is kept at 1 px or more and within the work area.",
      "On Windows, the position is clamped into the work area of the monitor the new rectangle lands on, so the window can move to another monitor but not behind the taskbar.",
      "On macOS and Linux, the position is not clamped. On macOS, `x` and `y` are relative to the screen the window is on, as `getBounds()` reports them.",
      "An animated change lays out the content at each intermediate size. Use animate: true for 180 ms with ease-in-out, or provide duration in milliseconds and easing. Move and resize events reflect actual changes during the glide and at its final rectangle. A new bounds request replaces the running glide.",
    ],
    examples: [
      {
        title: "Dock the window to the right half of the screen",
        lang: "ts",
        code: `${IMPORT}

export function dockRight() {
  if (appWindow.isMaximized()) appWindow.unmaximize();
  appWindow.setBounds({
    x: screen.availLeft + screen.availWidth / 2,
    y: screen.availTop,
    width: screen.availWidth / 2,
    height: screen.availHeight,
  });
}`,
      },
    ],
    edgeCases: ["`getBounds()` can return the old geometry until the next frame."],
    limitations: [WAYLAND_NO_POSITION],
    related: ["get-bounds", "set-content-bounds", "move-to", "center"],
  },
  {
    slug: "set-content-bounds",
    name: "appWindow.setContentBounds()",
    kind: "method",
    summary: "Moves or resizes the main window so its client area has the given rectangle, in CSS px.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.setContentBounds(rect: Partial<SkWindowRect>): void",
    params: [
      { name: "rect.x", type: "number", description: "Left edge of the client area.", optional: true },
      { name: "rect.y", type: "number", description: "Top edge of the client area.", optional: true },
      { name: "rect.width", type: "number", description: "Client width.", optional: true },
      { name: "rect.height", type: "number", description: "Client height.", optional: true },
    ],
    returns: "`undefined`.",
    notes: [
      "The size is applied asynchronously by the OS, so read it back on the next `resize` event.",
      "Clamping is the same as for `setBounds()`.",
    ],
    examples: [
      {
        title: "Fit the window to a fixed-size game canvas",
        lang: "ts",
        code: `${IMPORT}

export function fitTo(canvas: HTMLCanvasElement) {
  appWindow.setContentBounds({ width: canvas.width, height: canvas.height + 32 });
}`,
        note: "The 32 px make room for a custom title bar above the canvas.",
      },
    ],
    limitations: [WAYLAND_NO_POSITION],
    related: ["set-bounds", "get-content-bounds", "resize-to"],
  },
  {
    slug: "title",
    name: "appWindow.title",
    kind: "property",
    summary: "Reads or sets the main window's title, the same text as `document.title`.",
    platforms: all(OK),
    syntax: "appWindow.title: string",
    notes: [
      "The getter returns the text with whitespace collapsed and trimmed. The OS receives the text as written.",
      'The initial value is `window.title` from `sk.config.json`, or `"Skathi"`.',
      "It is reactive, unlike `document.title`.",
    ],
    examples: [
      {
        title: "Show the open document in the title",
        lang: "vue",
        code: `<script setup lang="ts">
import { watchEffect } from "vue";
${IMPORT}

const props = defineProps<{ fileName: string; dirty: boolean }>();

watchEffect(() => {
  appWindow.title = (props.dirty ? "• " : "") + props.fileName + " - Writer";
});
</script>

<template>
  <slot />
</template>`,
      },
    ],
    edgeCases: ["There is no event when the title changes."],
    related: ["document-title", "config-window-title", "sk-window-title"],
  },
  {
    slug: "backdrop",
    name: "appWindow.backdrop",
    kind: "property",
    summary: "Reads or sets the material drawn behind the main window's transparent content, such as Mica or Acrylic.",
    platforms: os(OK, PART, PART),
    syntax: 'appWindow.backdrop: "mica" | "mica-alt" | "acrylic" | "transparent" | "none"',
    values: [
      v("mica", OK, "The default. Tinted by the desktop wallpaper."),
      v("mica-alt", OK, "A stronger Mica tint, for tabbed windows. `tabbed` is accepted as a synonym."),
      v("acrylic", OK, "A blur of what is behind the window. `blur` is accepted as a synonym."),
      v("transparent", PART, "No material; the desktop shows through. On Linux it depends on the compositor."),
      v("none", OK, "An opaque window. `opaque` is accepted as a synonym."),
    ],
    notes: [
      "The getter returns the resolved name: set `blur` and read `acrylic`.",
      "On Windows before build 22621, `mica` and `mica-alt` fall back to `acrylic` with an advisory, and read back as `acrylic`.",
      "An unknown name changes nothing and prints an advisory.",
      "It is kept while fullscreen suspends it. `backdropchange` fires when it changes.",
      "It applies to the main window only. A pop-out draws its own `backdrop` prop.",
    ],
    examples: [
      {
        title: "Let the user pick the window material",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}

const materials = ["mica", "mica-alt", "acrylic", "none"];
</script>

<template>
  <label>
    Window material
    <select :value="appWindow.backdrop" @change="appWindow.backdrop = ($event.target as HTMLSelectElement).value">
      <option v-for="m in materials" :key="m" :value="m">{{ m }}</option>
    </select>
  </label>
</template>`,
      },
    ],
    edgeCases: ["The material shows only where the app's own background is transparent."],
    limitations: [
      "On macOS, `mica`, `mica-alt` and `acrylic` all draw the same behind-window blur; the getter still returns the name asked for.",
      "On Linux, `mica`, `mica-alt` and `acrylic` draw an opaque fill (`#202020` dark, `#f3f3f3` light), and `effectiveBackdrop` reads `none`.",
    ],
    related: ["effective-backdrop", "event-backdropchange", "config-window-backdrop", "sk-window-backdrop", "theme"],
    aliases: ["mica", "acrylic", "vibrancy", "material"],
  },
  {
    slug: "effective-backdrop",
    name: "appWindow.effectiveBackdrop",
    kind: "property",
    summary: "Reads the material the main window actually draws, which can differ from the one asked for.",
    platforms: all(OK),
    syntax: "readonly appWindow.effectiveBackdrop: string",
    notes: [
      "It equals `backdrop`, except `none` when no material is drawn: in macOS native fullscreen, and always on Linux for `mica`, `mica-alt` and `acrylic`.",
      "`backdropchange` fires when it changes. It is reactive.",
    ],
    examples: [
      {
        title: "Paint a solid background when no material is drawn",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}
</script>

<template>
  <div class="shell" :class="{ solid: appWindow.effectiveBackdrop === 'none' }">
    <slot />
  </div>
</template>

<style scoped>
.shell.solid {
  background-color: #1f1f1f;
}
</style>`,
      },
    ],
    related: ["backdrop", "event-backdropchange"],
  },
  {
    slug: "theme",
    name: "appWindow.theme",
    kind: "property",
    summary: "Forces the app's color scheme to light or dark, or hands it back to the system.",
    platforms: os(OK, PART, PART),
    syntax: 'appWindow.theme: "system" | "light" | "dark"',
    values: [
      v("system", OK, "The default. Follows the OS setting."),
      v("light", PART, "Forces light. On macOS and Linux, only `prefers-color-scheme` follows."),
      v("dark", PART, "Forces dark. On macOS and Linux, only `prefers-color-scheme` follows."),
    ],
    notes: [
      "It drives `prefers-color-scheme` for the whole app, pop-outs included. On Windows it also sets the dark mode of every window, which tints Mica and the OS caption.",
      "A forced theme stays when the OS theme changes.",
      "It starts at `system` on each launch and is not saved; store the user's choice yourself.",
      "Names are case-sensitive: `Dark` prints an advisory and changes nothing.",
      "Like Electron's `nativeTheme.themeSource`.",
    ],
    examples: [
      {
        title: "A theme setting that survives restarts",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}

appWindow.theme = localStorage.getItem("theme") ?? "system";

function pick(theme: string) {
  appWindow.theme = theme;
  localStorage.setItem("theme", theme);
}
</script>

<template>
  <div role="radiogroup" class="theme-picker">
    <button v-for="t in ['system', 'light', 'dark']" :key="t" :aria-pressed="appWindow.theme === t" @click="pick(t)">
      {{ t }}
    </button>
  </div>
</template>`,
      },
    ],
    edgeCases: ['There is no `themechange` event. Listen to `matchMedia("(prefers-color-scheme: dark)")` instead.'],
    limitations: [
      "On macOS and Linux, only `prefers-color-scheme` follows. The native title bar, the traffic lights and the vibrancy keep the OS appearance.",
    ],
    related: ["match-media", "backdrop", "event-change"],
    aliases: ["dark mode", "nativeTheme", "color scheme"],
  },
  {
    slug: "always-on-top",
    name: "appWindow.alwaysOnTop",
    kind: "property",
    summary: "Reads or sets whether the main window stays above other windows.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.alwaysOnTop: boolean",
    notes: [
      "The window stays above other windows without being activated.",
      "The initial value is `window.alwaysOnTop` from `sk.config.json`. It is reactive. Pair it with visibleOnAllWorkspaces for a player over fullscreen Spaces on macOS.",
    ],
    examples: [
      {
        title: "A pin button for a mini player",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}
</script>

<template>
  <button
    class="pin"
    :aria-pressed="appWindow.alwaysOnTop"
    @click="appWindow.alwaysOnTop = !appWindow.alwaysOnTop"
  >
    {{ appWindow.alwaysOnTop ? "Unpin" : "Keep on top" }}
  </button>
</template>`,
      },
    ],
    limitations: ["On Linux under Wayland, setting `true` prints an advisory, and the getter stays `false`."],
    related: ["config-window-always-on-top", "sk-window-always-on-top"],
    aliases: ["topmost", "pin"],
  },
  {
    slug: "scale-factor",
    name: "appWindow.scaleFactor",
    kind: "property",
    summary: "Reads how many device pixels make one CSS pixel on the main window's display.",
    platforms: all(OK),
    syntax: "readonly appWindow.scaleFactor: number",
    notes: [
      "It equals `devicePixelRatio`. It is `1` until the first frame.",
      "Moving the window to a display with another scale fires `resize` on `window` and `geometrychange`.",
      "It is reactive.",
    ],
    examples: [
      {
        title: "Show the display scale in a settings page",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}
</script>

<template>
  <p>Display scale: {{ Math.round(appWindow.scaleFactor * 100) }} %</p>
</template>`,
      },
    ],
    related: ["device-pixel-ratio", "sk-window-scale", "event-resize"],
    aliases: ["DPI", "dpr"],
  },
  {
    slug: "document-title",
    name: "document.title",
    kind: "property",
    summary: "Reads or sets the main window's title.",
    platforms: all(OK),
    mdn: `${MDN}API/Document/title`,
    syntax: "document.title: string",
    notes: [
      "The same value as `appWindow.title`. Whitespace is collapsed on read, and the OS receives the text as written.",
      "It is not reactive; read `appWindow.title` in templates.",
    ],
    examples: [
      {
        title: "Show the unread count in the title",
        lang: "ts",
        code: `export function showUnread(count: number) {
  document.title = count > 0 ? "(" + count + ") Mail" : "Mail";
}`,
      },
    ],
    related: ["title", "config-window-title"],
  },
  {
    slug: "inner-width",
    name: "window.innerWidth / innerHeight",
    kind: "property",
    summary: "Reads the size of the area the app draws in, in CSS px.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/innerWidth`,
    syntax: "readonly window.innerWidth: number\nreadonly window.innerHeight: number",
    notes: [
      "It is the box of the app's root, without the host's temporary title bar strip. Before the first frame it equals the client area.",
      "It always describes the main window, also from code in a pop-out. A pop-out's size comes from `<SkWindow @resize>`.",
      "It is not reactive. Listen to `resize` on `window`.",
    ],
    examples: [
      {
        title: "Keep a canvas the size of the window",
        lang: "ts",
        code: `export function fillWindow(canvas: HTMLCanvasElement) {
  const fit = () => {
    canvas.width = Math.round(innerWidth * devicePixelRatio);
    canvas.height = Math.round(innerHeight * devicePixelRatio);
  };
  fit();
  addEventListener("resize", fit);
}`,
      },
    ],
    related: ["outer-width", "event-resize", "device-pixel-ratio", "sk-window-width"],
    aliases: ["innerHeight", "viewport size"],
  },
  {
    slug: "outer-width",
    name: "window.outerWidth / outerHeight",
    kind: "property",
    summary: "Reads the size of the main window, frame included, in CSS px.",
    platforms: os(OK, PART, PART),
    mdn: `${MDN}API/Window/outerWidth`,
    syntax: "readonly window.outerWidth: number\nreadonly window.outerHeight: number",
    notes: ["The values are rounded. They are not reactive; read `appWindow.getBounds()` for that."],
    examples: [
      {
        title: "Log the window size at startup",
        lang: "ts",
        code: `addEventListener("load", () => {
  console.info("window " + outerWidth + "x" + outerHeight + ", content " + innerWidth + "x" + innerHeight);
});`,
      },
    ],
    limitations: ["On macOS and Linux, it equals the content bounds."],
    related: ["inner-width", "get-bounds", "screen-x"],
    aliases: ["outerHeight"],
  },
  {
    slug: "screen-x",
    name: "window.screenX / screenY",
    kind: "property",
    summary: "Reads the position of the main window's top-left corner on the screen, in CSS px.",
    platforms: os(OK, PART, PART),
    mdn: `${MDN}API/Window/screenX`,
    syntax:
      "readonly window.screenX: number\nreadonly window.screenY: number\nreadonly window.screenLeft: number\nreadonly window.screenTop: number",
    notes: ["`screenLeft` and `screenTop` are the same values. They come from the last frame and are not reactive."],
    examples: [
      {
        title: "Open a pop-out next to the main window",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
import { SkWindow } from "@skathi-native/runtime";

const open = ref(false);
const at = ref({ x: 0, y: 0, width: 280, height: 400 });

function openInspector() {
  at.value = { ...at.value, x: screenX + outerWidth + 8, y: screenY };
  open.value = true;
}
</script>

<template>
  <button @click="openInspector">Inspector</button>
  <SkWindow v-if="open" title="Inspector" :bounds="at" @close="open = false"><slot /></SkWindow>
</template>`,
      },
    ],
    limitations: [
      "On macOS, the position is relative to the screen the window is on, and it is the content bounds.",
      "On Linux under Wayland, the position is not reported reliably.",
    ],
    related: ["get-bounds", "move-to", "event-move"],
    aliases: ["screenY", "screenLeft", "screenTop"],
  },
  {
    slug: "device-pixel-ratio",
    name: "window.devicePixelRatio",
    kind: "property",
    summary: "Reads how many device pixels make one CSS pixel on the main window's display.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/devicePixelRatio`,
    syntax: "readonly window.devicePixelRatio: number",
    notes: ["It equals `appWindow.scaleFactor`. A change fires `resize` on `window`."],
    examples: [
      {
        title: "Draw sharp lines on a canvas",
        lang: "ts",
        code: `export function setupCanvas(canvas: HTMLCanvasElement, width: number, height: number) {
  canvas.width = Math.round(width * devicePixelRatio);
  canvas.height = Math.round(height * devicePixelRatio);
  canvas.style.width = width + "px";
  canvas.style.height = height + "px";
  canvas.getContext("2d")!.scale(devicePixelRatio, devicePixelRatio);
}`,
      },
    ],
    related: ["scale-factor", "inner-width"],
  },
  {
    slug: "resize-to",
    name: "window.resizeTo()",
    kind: "function",
    summary: "Resizes the main window to the given outer size, frame included.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/resizeTo`,
    syntax: "window.resizeTo(width: number, height: number): void",
    params: [
      { name: "width", type: "number", description: "Outer width in CSS px." },
      { name: "height", type: "number", description: "Outer height in CSS px." },
    ],
    returns: "`undefined`.",
    notes: [
      "It is `appWindow.setBounds({ width, height })`: the size is the one `getBounds()` reports, as in a browser.",
      "The size is clamped to the work area.",
      "To size the client area instead, call `appWindow.setContentBounds({ width, height })`.",
    ],
    examples: [
      {
        title: "Grow the window when a details panel opens",
        lang: "ts",
        code: `export function openDetails(panel: HTMLElement) {
  panel.hidden = false;
  if (outerWidth < 960) resizeTo(960, outerHeight);
}`,
      },
    ],
    related: ["set-bounds", "set-content-bounds", "resize-by", "outer-width"],
  },
  {
    slug: "resize-by",
    name: "window.resizeBy()",
    kind: "function",
    summary: "Grows or shrinks the main window by the given amounts.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/resizeBy`,
    syntax: "window.resizeBy(dx: number, dy: number): void",
    params: [
      { name: "dx", type: "number", description: "Change in width, in CSS px." },
      { name: "dy", type: "number", description: "Change in height, in CSS px." },
    ],
    returns: "`undefined`.",
    notes: ["It adds `dx` and `dy` to the outer size, as `resizeTo()` sets it. The size is clamped to the work area."],
    examples: [
      {
        title: "Widen the window by a sidebar's width",
        lang: "ts",
        code: `export function showSidebar(sidebar: HTMLElement) {
  sidebar.hidden = false;
  resizeBy(sidebar.offsetWidth, 0);
}`,
      },
    ],
    related: ["resize-to", "set-bounds"],
  },
  {
    slug: "move-to",
    name: "window.moveTo()",
    kind: "function",
    summary: "Moves the main window's top-left corner to a screen position.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    mdn: `${MDN}API/Window/moveTo`,
    syntax: "window.moveTo(x: number, y: number): void",
    params: [
      { name: "x", type: "number", description: "Left edge of the outer window, in CSS px." },
      { name: "y", type: "number", description: "Top edge of the outer window, in CSS px." },
    ],
    returns: "`undefined`.",
    notes: [
      "It is `appWindow.setBounds({ x, y })`.",
      "On Windows, the position is clamped into the work area of the monitor it lands on, so the window can move to another monitor.",
    ],
    examples: [
      {
        title: "Snap the window to the top-left of the work area",
        lang: "ts",
        code: `export function snapTopLeft() {
  moveTo(screen.availLeft, screen.availTop);
}`,
      },
    ],
    limitations: ["On Linux under Wayland, it does nothing and prints an advisory: the compositor places the window."],
    related: ["set-bounds", "move-by", "screen"],
  },
  {
    slug: "move-by",
    name: "window.moveBy()",
    kind: "function",
    summary: "Moves the main window by the given offsets.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    mdn: `${MDN}API/Window/moveBy`,
    syntax: "window.moveBy(dx: number, dy: number): void",
    params: [
      { name: "dx", type: "number", description: "Horizontal offset in CSS px." },
      { name: "dy", type: "number", description: "Vertical offset in CSS px." },
    ],
    returns: "`undefined`.",
    notes: ["It is `appWindow.setBounds({ x: screenX + dx, y: screenY + dy })`."],
    examples: [
      {
        title: "Nudge the window with Ctrl and the arrow keys",
        lang: "ts",
        code: `const steps: Record<string, [number, number]> = {
  ArrowLeft: [-10, 0],
  ArrowRight: [10, 0],
  ArrowUp: [0, -10],
  ArrowDown: [0, 10],
};

addEventListener("keydown", (e) => {
  const step = e.ctrlKey ? steps[e.key] : undefined;
  if (step) moveBy(step[0], step[1]);
});`,
      },
    ],
    limitations: ["On Linux under Wayland, it does nothing and prints an advisory: the compositor places the window."],
    related: ["move-to", "set-bounds", "screen-x"],
  },
  {
    slug: "window-close",
    name: "window.close()",
    kind: "function",
    summary: "Closes the main window and quits the app.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/close`,
    syntax: "window.close(): void",
    returns: "`undefined`.",
    notes: [
      "The same function as `appWindow.close()`: `pagehide` fires, then the app quits without dispatching `close-requested`.",
      "There is no opener rule as in browsers: it always closes.",
    ],
    examples: [
      {
        title: "A Quit item in the app's own menu",
        lang: "vue",
        code: `<script setup lang="ts">
const emit = defineEmits<{ new: [] }>();
const quit = () => window.close();
</script>

<template>
  <menu class="file-menu">
    <li><button @click="emit('new')">New</button></li>
    <li><button @click="quit">Quit</button></li>
  </menu>
</template>`,
      },
    ],
    related: ["close", "event-pagehide"],
  },
  {
    slug: "window-focus",
    name: "window.focus()",
    kind: "function",
    summary: "Asks for the user's attention, or raises the main window when the app opts in.",
    platforms: os(OK, OK, PART),
    mdn: `${MDN}API/Window/focus`,
    syntax: "window.focus(): void",
    returns: "`undefined`.",
    notes: [
      "By default it does not raise the window: it flashes the taskbar button or bounces the Dock icon, and prints an advisory once.",
      'With `"focus": "raise"` in the `window` block of `sk.config.json`, it activates and raises the window.',
      "To always raise, call `appWindow.show()`.",
    ],
    examples: [
      {
        title: "Flag the window when a build fails",
        lang: "ts",
        code: `export function onBuildResult(ok: boolean) {
  if (!ok && !document.hasFocus()) window.focus();
}`,
      },
    ],
    limitations: [
      "On macOS, it bounces the Dock icon only when the app is not already active.",
      "On Linux under Wayland, the attention request does nothing. Under X11 it sets the urgency hint.",
    ],
    related: ["config-window-focus", "show", "document-has-focus"],
    aliases: ["requestAttention", "flashFrame"],
  },
  {
    slug: "window-blur",
    name: "window.blur()",
    kind: "function",
    summary: "Would send the main window to the back; it does nothing, as in every current browser.",
    platforms: all(NO),
    mdn: `${MDN}API/Window/blur`,
    syntax: "window.blur(): void",
    returns: "`undefined`.",
    workaround: "Minimize the window with `appWindow.minimize()`, or hide it with `appWindow.hide()`.",
    related: ["window-focus", "minimize"],
  },
  {
    slug: "document-has-focus",
    name: "document.hasFocus()",
    kind: "method",
    summary: "Returns whether the main window is the active window.",
    platforms: all(OK),
    mdn: `${MDN}API/Document/hasFocus`,
    syntax: "document.hasFocus(): boolean",
    returns: "`true` while the main window is active.",
    notes: ["It reads the state of the last frame. It is not reactive; use `appWindow.isFocused()` in templates."],
    examples: [
      {
        title: "Mark a message read only once the user is looking",
        lang: "ts",
        code: `export function whenSeen(markRead: () => void) {
  if (document.hasFocus()) markRead();
  else addEventListener("focus", markRead, { once: true });
}`,
      },
    ],
    related: ["is-focused", "event-focus", "event-blur"],
  },
  {
    slug: "document-visibility-state",
    name: "document.visibilityState / hidden",
    kind: "property",
    summary: "Reads whether the main window can be seen: shown, or hidden, minimized or covered.",
    platforms: os(OK, PART, PART),
    platformSince: MAC_LINUX_0_3,
    mdn: `${MDN}API/Document/visibilityState`,
    syntax: 'readonly document.visibilityState: "visible" | "hidden"\nreadonly document.hidden: boolean',
    notes: [
      "On Windows, the page is hidden while the window is hidden, minimized, on another virtual desktop, or fully covered by other windows. A shown window in the background stays visible. A fully covered window is hidden even while it has the focus, and every window is hidden while the system has turned the display off.",
      "On macOS and Linux, the page is hidden while the window is hidden or minimized.",
      "It is set before the app's code runs.",
      "While hidden, `requestAnimationFrame` callbacks do not run and animations pause.",
      "On Windows, set `SK_OCCLUSION=0` to stop treating a covered window as hidden.",
    ],
    examples: [
      {
        title: "Pause a live chart while nobody can see it",
        lang: "ts",
        code: `export function liveChart(poll: () => void) {
  let timer = 0;
  const sync = () => {
    clearInterval(timer);
    if (!document.hidden) timer = setInterval(poll, 1000);
  };
  sync();
  document.addEventListener("visibilitychange", sync);
}`,
      },
    ],
    limitations: [
      "On macOS and Linux, a window that is covered or on another desktop stays `visible`.",
      "On Linux under Wayland, the page is always `visible`: the compositor reports neither minimized nor hidden.",
    ],
    related: ["event-visibilitychange", "is-visible", "is-minimized", "hide"],
    aliases: ["hidden", "Page Visibility"],
  },
  {
    slug: "screen",
    name: "window.screen",
    kind: "property",
    summary: "Describes the display the main window is on: its size and its work area, in CSS px.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/screen`,
    syntax:
      "readonly window.screen: Screen // width, height, availWidth, availHeight, availLeft, availTop, colorDepth, pixelDepth, isExtended, orientation",
    notes: [
      "`availLeft`, `availTop`, `availWidth` and `availHeight` are the work area, without the taskbar or the Dock.",
      "`isExtended` is `true` when more than one display is connected.",
      "`colorDepth` and `pixelDepth` are `24`. The values are exact.",
      "It follows the main window to another display, and a change of the taskbar's size.",
    ],
    examples: [
      {
        title: "Open at a size that fits small screens",
        lang: "ts",
        code: `${IMPORT}

const width = Math.min(1280, screen.availWidth - 80);
const height = Math.min(800, screen.availHeight - 80);
appWindow.setContentBounds({ width, height });`,
      },
    ],
    edgeCases: ["`screen` never fires `change`."],
    workaround:
      'There is no list of displays. For a window on another monitor, use `<SkWindow cover="monitor" :monitor>` on Windows.',
    related: ["screen-orientation", "set-bounds", "sk-window-monitor"],
  },
  {
    slug: "screen-orientation",
    name: "screen.orientation",
    kind: "property",
    summary: "Reports whether the main window's display is landscape or portrait.",
    platforms: all(OK),
    mdn: `${MDN}API/Screen/orientation`,
    syntax:
      'readonly screen.orientation: { type: "landscape-primary" | "portrait-primary"; angle: 0; lock(): Promise<never>; unlock(): void }',
    notes: [
      "`type` is `landscape-primary` when the display is at least as wide as it is tall. `angle` is always `0`.",
      "`lock()` rejects with `NotSupportedError`, as in desktop browsers.",
    ],
    examples: [
      {
        title: "Stack panes on a portrait monitor",
        lang: "vue",
        code: `<script setup lang="ts">
const portrait = screen.orientation.type.startsWith("portrait");
</script>

<template>
  <div class="panes" :class="portrait ? 'stacked' : 'side-by-side'"><slot /></div>
</template>`,
      },
    ],
    edgeCases: ["It never fires `change`; read it again on `resize`."],
    related: ["screen"],
  },
  {
    slug: "match-media",
    name: "window.matchMedia()",
    kind: "function",
    summary: "Evaluates a media query against the main window and reports changes to its result.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/matchMedia`,
    syntax: "window.matchMedia(query: string): MediaQueryList",
    params: [{ name: "query", type: "string", description: "A media query, such as `(prefers-color-scheme: dark)`." }],
    returns: "A `MediaQueryList` with `matches`, `media`, `onchange`, `addListener` and `removeListener`.",
    notes: [
      "It uses the same evaluator as `@media`, against the main window, also from code in a pop-out.",
      "Inside a `resize` handler, it already answers for the new size.",
      "A query the evaluator cannot run answers `false` and prints an advisory. See [matchMedia](/web-platform/match-media/).",
    ],
    examples: [
      {
        title: "Switch to a compact layout below 700 px",
        lang: "vue",
        code: `<script setup lang="ts">
import { onUnmounted, ref } from "vue";

const narrow = matchMedia("(max-width: 700px)");
const compact = ref(narrow.matches);
const update = (e: MediaQueryListEvent) => (compact.value = e.matches);
narrow.addEventListener("change", update);
onUnmounted(() => narrow.removeEventListener("change", update));
</script>

<template>
  <div :class="compact ? 'layout-compact' : 'layout-wide'"><slot /></div>
</template>`,
      },
    ],
    edgeCases: ["Every list ever created is kept and re-evaluated each frame. Create them once, not in a loop."],
    related: ["event-change", "event-resize", "theme", "display-mode"],
  },
  {
    slug: "request-fullscreen",
    name: "Element.requestFullscreen()",
    kind: "method",
    summary: "Shows one element fullscreen in the window that paints it.",
    platforms: all(OK),
    mdn: `${MDN}API/Element/requestFullscreen`,
    syntax: "element.requestFullscreen(options?: FullscreenOptions): Promise<void>",
    params: [
      {
        name: "options",
        type: "FullscreenOptions",
        description: "`keyboardLock` and `screen` print an advisory and are ignored; `navigationUI` is ignored.",
        optional: true,
      },
    ],
    returns: "A promise that resolves once `fullscreenchange` has fired and the layout fills the monitor.",
    notes: [
      "It puts the window that paints the element fullscreen: the main window, or the `<SkWindow>` the element is in.",
      "No user gesture is needed, unlike in browsers.",
      "Escape leaves element fullscreen and does not reach the app. Removing the element from the page also leaves it.",
      "If the window was already fullscreen through `setFullscreen()`, leaving keeps the window fullscreen.",
      "See the [Fullscreen API](/web-platform/fullscreen-api/).",
    ],
    examples: [
      {
        title: "A video player with a fullscreen button",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const player = ref<HTMLElement | null>(null);

function toggle() {
  if (document.fullscreenElement) void document.exitFullscreen();
  else void player.value?.requestFullscreen();
}
</script>

<template>
  <div ref="player" class="player">
    <video src="assets/intro.mp4" controls />
    <button class="fs" @click="toggle">Fullscreen</button>
  </div>
</template>`,
      },
    ],
    edgeCases: [
      "It rejects with `TypeError` when the element is not in the page, its window has closed, or another window already holds fullscreen.",
      "Asking again for the current element resolves at once.",
      "`fullscreenerror` never fires; the promise rejects instead.",
    ],
    related: ["exit-fullscreen", "fullscreen-element", "event-document-fullscreenchange", "set-fullscreen"],
  },
  {
    slug: "exit-fullscreen",
    name: "document.exitFullscreen()",
    kind: "method",
    summary: "Takes the current element out of fullscreen.",
    platforms: all(OK),
    mdn: `${MDN}API/Document/exitFullscreen`,
    syntax: "document.exitFullscreen(): Promise<void>",
    returns: "A promise that resolves when the element has left fullscreen, or at once if nothing is fullscreen.",
    notes: [
      "It leaves one level: the element requested before it becomes fullscreen again. When none is left, the window leaves fullscreen, unless `setFullscreen(true)` had made it fullscreen before.",
      "On Windows, the windowed rectangle is restored exactly.",
    ],
    examples: [
      {
        title: "Leave fullscreen from a Done button",
        lang: "vue",
        code: `<script setup lang="ts">
const done = () => document.exitFullscreen();
</script>

<template>
  <div class="slideshow">
    <slot />
    <button class="done" @click="done">Done</button>
  </div>
</template>`,
      },
    ],
    related: ["request-fullscreen", "fullscreen-element"],
  },
  {
    slug: "fullscreen-element",
    name: "document.fullscreenElement",
    kind: "property",
    summary: "Reads the element that is fullscreen, or `null`.",
    platforms: all(OK),
    mdn: `${MDN}API/Document/fullscreenElement`,
    syntax: "readonly document.fullscreenElement: Element | null",
    notes: [
      "It is `null` when the window is fullscreen through `appWindow.setFullscreen(true)`.",
      "It becomes `null` when the OS takes the main window out of fullscreen, as the macOS green button does.",
    ],
    examples: [
      {
        title: "Toggle fullscreen for a viewer",
        lang: "ts",
        code: `export function toggleFullscreen(viewer: HTMLElement) {
  if (document.fullscreenElement === viewer) return document.exitFullscreen();
  return viewer.requestFullscreen();
}`,
      },
    ],
    related: ["request-fullscreen", "exit-fullscreen", "is-fullscreen"],
  },
  {
    slug: "fullscreen-enabled",
    name: "document.fullscreenEnabled",
    kind: "property",
    summary: "Reports whether elements can go fullscreen; it is always `true`.",
    platforms: all(OK),
    mdn: `${MDN}API/Document/fullscreenEnabled`,
    syntax: "readonly document.fullscreenEnabled: true",
    examples: [
      {
        title: "Show the fullscreen button only where it works",
        lang: "vue",
        code: `<script setup lang="ts">
const canFullscreen = document.fullscreenEnabled;
const emit = defineEmits<{ fullscreen: [] }>();
</script>

<template>
  <button v-if="canFullscreen" class="fs" @click="emit('fullscreen')">Fullscreen</button>
</template>`,
        note: "The same component runs in a browser preview, where the value can be `false`.",
      },
    ],
    related: ["request-fullscreen"],
  },
]);

const HOST_EVENT =
  "A plain `Event` with no payload; read the new state from `appWindow`, whose getters are already current in the handler.";

const windowEvents = section("Window events", [
  {
    slug: "event-fullscreenwillchange",
    name: "fullscreenwillchange (appWindow)",
    kind: "event",
    summary: "Reports the direction of a fullscreen transition before its completion.",
    platforms: all(OK),
    since: "0.6.0",
    syntax: 'appWindow.addEventListener("fullscreenwillchange", handler)',
    notes: [
      "Read event.detail.fullscreen for the destination. On macOS the source is AppKit’s will-enter or will-exit notification, including transitions initiated by the green button. On Windows and Linux the host publishes it before fullscreenchange in the same state update.",
    ],
  },
  {
    slug: "event-fullscreencancel",
    name: "fullscreencancel (appWindow)",
    kind: "event",
    summary: "Reports a native fullscreen transition cancelled by AppKit.",
    platforms: os(NO, OK, NO),
    since: "0.6.0",
    syntax: 'appWindow.addEventListener("fullscreencancel", handler)',
    notes: [
      "event.detail.fullscreen is the requested destination. The existing windowed or fullscreen state remains in effect.",
    ],
  },
  {
    slug: "event-close-requested",
    name: "close-requested",
    kind: "event",
    summary: "Fires on `appWindow` when the user asks to close the main window; cancel it to keep the window open.",
    platforms: all(OK),
    syntax: 'appWindow.addEventListener("close-requested", (event: Event) => void)',
    notes: [
      "It fires for the title bar's close button, a `--window-control: close` element and the `titleBarOverlay` close button, Alt+F4, the taskbar's Close, macOS's red button and a `WM_CLOSE` from another process.",
      "`event.preventDefault()` keeps the window open. It works also when the user has not interacted with the app yet, so an app can hide to the tray.",
      "When no listener cancels it, `beforeunload` fires, then `pagehide`, then the app quits.",
      'It does not fire for `appWindow.close()`, `window.close()`, a tray `role: "quit"` item, the session ending, or a pop-out\'s close.',
      "The veto is read synchronously, so decide in the handler and show any dialog afterwards.",
      "Like Tauri's `onCloseRequested` or Electron's `close` event.",
    ],
    examples: [
      {
        title: "Hide to the tray instead of closing",
        lang: "ts",
        code: `import { appTray, appWindow } from "skathi:app";

appWindow.addEventListener("close-requested", (e) => {
  if (!appTray.available) return;
  e.preventDefault();
  appWindow.hide();
});`,
      },
      {
        title: "Confirm before discarding unsaved work",
        lang: "ts",
        code: `import { ref } from "vue";
${IMPORT}

export const dirty = ref(false);
export const confirmClose = ref(false);

appWindow.addEventListener("close-requested", (e) => {
  if (!dirty.value) return;
  e.preventDefault();
  confirmClose.value = true;
});

export const discardAndClose = () => appWindow.close();`,
        note: "Show a dialog while `confirmClose` is `true`; its Discard button calls `discardAndClose`.",
      },
    ],
    edgeCases: ["A listener that throws counts as no veto, and the app closes."],
    related: ["close", "hide", "event-beforeunload", "event-pagehide", "app-tray"],
    aliases: ["onCloseRequested", "close event", "prevent close"],
  },
  {
    slug: "event-beforeunload",
    name: "beforeunload",
    kind: "event",
    summary: "Fires on `window` before the app closes; cancelling it can block the close, but no dialog is shown.",
    platforms: all(PART),
    mdn: `${MDN}API/Window/beforeunload_event`,
    syntax: 'addEventListener("beforeunload", (event: Event) => void)',
    notes: [
      "It fires only when `close-requested` was not cancelled.",
      "`preventDefault()` blocks the close only after the user has pressed a key or a mouse button in the app, as in Chrome.",
      "If it blocks the close and nothing listens to `close-requested`, an advisory explains that the close button now does nothing.",
      "It does not fire for `appWindow.close()` or `window.close()`.",
    ],
    examples: [
      {
        title: "Flush a draft as the app closes",
        lang: "ts",
        code: `export function keepDraft(editor: HTMLTextAreaElement) {
  addEventListener("beforeunload", () => {
    localStorage.setItem("draft", editor.value);
  });
}`,
        note: "`pagehide` covers more close paths; prefer it for saving.",
      },
    ],
    edgeCases: ["No confirmation prompt exists, and `returnValue` has no effect."],
    workaround:
      "To ask the user, cancel `close-requested` on `appWindow`, show the app's own dialog, and call `appWindow.close()` when they confirm.",
    related: ["event-close-requested", "event-pagehide", "close"],
  },
  {
    slug: "event-pagehide",
    name: "pagehide",
    kind: "event",
    summary: "Fires on `window` once, as the app closes, before its windows disappear.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/pagehide_event`,
    syntax: 'addEventListener("pagehide", (event: Event) => void)',
    notes: [
      'It fires on every way the app closes or quits: `appWindow.close()`, `window.close()`, a user close that nobody cancelled, a tray `role: "quit"` item, the embedder closing the app, the session ending, and any other quit of the app.',
      "It fires once per run, also when several of these happen.",
      "Writes to `localStorage` made in the handler reach disk.",
    ],
    examples: [
      {
        title: "Save the open tabs on exit",
        lang: "ts",
        code: `export function persistTabs(tabs: () => string[]) {
  addEventListener("pagehide", () => {
    localStorage.setItem("open-tabs", JSON.stringify(tabs()));
  });
}`,
      },
    ],
    related: ["close", "event-close-requested", "event-beforeunload", "event-unload"],
  },
  {
    slug: "event-unload",
    name: "unload",
    kind: "event",
    summary: "The legacy page-unload event; the listener is accepted and never called.",
    platforms: all(NO),
    mdn: `${MDN}API/Window/unload_event`,
    syntax: 'addEventListener("unload", (event: Event) => void)',
    notes: ["Browsers are removing the event as well."],
    workaround: "Listen to `pagehide`, or to `visibilitychange` for a window that goes to the background.",
    related: ["event-pagehide", "event-visibilitychange"],
  },
  {
    slug: "event-load",
    name: "DOMContentLoaded / load",
    kind: "event",
    summary: "Fire once on `window` after the app's code has run, before the first frame.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/load_event`,
    syntax:
      'addEventListener("load", (event: Event) => void)\naddEventListener("DOMContentLoaded", (event: Event) => void)',
    notes: ["Both fire once, after the app bundle has run. On Windows they fire before the window is first shown."],
    examples: [
      {
        title: "Measure startup time",
        lang: "ts",
        code: `addEventListener("load", () => {
  console.info("ready in " + Math.round(performance.now()) + " ms");
});`,
      },
    ],
    related: ["event-resize"],
    aliases: ["DOMContentLoaded", "onload"],
  },
  {
    slug: "event-resize",
    name: "resize",
    kind: "event",
    summary: "Fires on `window` and on `appWindow` when the main window's content size or display scale changes.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/resize_event`,
    syntax:
      'addEventListener("resize", (event: Event) => void)\nappWindow.addEventListener("resize", (event: Event) => void)',
    notes: [
      "It fires when `innerWidth`, `innerHeight` or `devicePixelRatio` changes, at most once a frame. On `window` it also fires once at the first frame.",
      "It fires on `window` before the other window events of the same frame, and on `appWindow` right after `visibilitychange`.",
      "`matchMedia()` in the handler already answers for the new size.",
      "A pop-out's size is `<SkWindow @resize>`.",
    ],
    examples: [
      {
        title: "Collapse the sidebar in a narrow window",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const collapsed = ref(innerWidth < 720);
const update = () => (collapsed.value = innerWidth < 720);

onMounted(() => addEventListener("resize", update));
onUnmounted(() => removeEventListener("resize", update));
</script>

<template>
  <aside class="sidebar" :class="{ collapsed }"><slot /></aside>
</template>`,
      },
    ],
    related: ["inner-width", "match-media", "sk-window-event-resize", "event-geometrychange"],
  },
  {
    slug: "event-focus",
    name: "focus",
    kind: "event",
    summary: "Fires on `window` and on `appWindow` when the main window becomes the active window.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/focus_event`,
    syntax:
      'addEventListener("focus", (event: Event) => void)\nappWindow.addEventListener("focus", (event: Event) => void)',
    notes: ["It fires on `window` first, then on `appWindow`.", "A pop-out's activation is `<SkWindow @focus>`."],
    examples: [
      {
        title: "Refresh the inbox when the user comes back",
        lang: "ts",
        code: `export function refreshOnReturn(refresh: () => void) {
  addEventListener("focus", refresh);
}`,
      },
    ],
    related: ["event-blur", "is-focused", "document-has-focus", "sk-window-event-focus"],
  },
  {
    slug: "event-blur",
    name: "blur",
    kind: "event",
    summary: "Fires on `window` and on `appWindow` when the main window stops being the active window.",
    platforms: all(OK),
    mdn: `${MDN}API/Window/blur_event`,
    syntax:
      'addEventListener("blur", (event: Event) => void)\nappWindow.addEventListener("blur", (event: Event) => void)',
    notes: [
      "It also fires when one of the app's own pop-outs becomes active.",
      "It fires on `window` first, then on `appWindow`.",
    ],
    examples: [
      {
        title: "Close an open menu when the window loses focus",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const open = ref(false);
const close = () => (open.value = false);

onMounted(() => addEventListener("blur", close));
onUnmounted(() => removeEventListener("blur", close));
</script>

<template>
  <button @click="open = !open">Menu</button>
  <ul v-if="open" class="menu"><slot /></ul>
</template>`,
      },
    ],
    related: ["event-focus", "is-focused", "sk-window-event-blur"],
  },
  {
    slug: "event-maximize",
    name: "maximize",
    kind: "event",
    summary: "Fires on `appWindow` when the main window becomes maximized.",
    platforms: all(OK),
    syntax: 'appWindow.addEventListener("maximize", (event: Event) => void)',
    notes: [
      HOST_EVENT,
      "A `geometrychange` follows in the same frame.",
      "On macOS, it fires when the window is zoomed.",
    ],
    examples: [
      {
        title: "Drop the window's rounded corners while maximized",
        lang: "ts",
        code: `${IMPORT}

const sync = () => document.documentElement.classList.toggle("maximized", appWindow.isMaximized());
appWindow.addEventListener("maximize", sync);
appWindow.addEventListener("unmaximize", sync);
sync();`,
      },
    ],
    related: ["event-unmaximize", "is-maximized", "maximize"],
  },
  {
    slug: "event-unmaximize",
    name: "unmaximize",
    kind: "event",
    summary: "Fires on `appWindow` when the main window leaves the maximized state.",
    platforms: all(OK),
    syntax: 'appWindow.addEventListener("unmaximize", (event: Event) => void)',
    notes: [HOST_EVENT, "On macOS, it fires when the window leaves the zoomed state."],
    examples: [
      {
        title: "Save the restored size after un-maximizing",
        lang: "ts",
        code: `${IMPORT}

appWindow.addEventListener("unmaximize", () => {
  localStorage.setItem("window-bounds", JSON.stringify(appWindow.getBounds()));
});`,
        note: "`getBounds()` can still read the maximized rectangle in this frame; save again on the next `move` or `resize` if exact values matter.",
      },
    ],
    related: ["event-maximize", "unmaximize", "is-maximized"],
  },
  {
    slug: "event-minimize",
    name: "minimize",
    kind: "event",
    summary: "Fires on `appWindow` when the main window is minimized.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: 'appWindow.addEventListener("minimize", (event: Event) => void)',
    notes: [HOST_EVENT, "It fires after `visibilitychange` in the same frame."],
    examples: [
      {
        title: "Pause playback when the window is minimized",
        lang: "ts",
        code: `${IMPORT}

export function pauseWhenMinimized(player: HTMLMediaElement) {
  appWindow.addEventListener("minimize", () => player.pause());
}`,
      },
    ],
    limitations: ["On Linux under Wayland, it never fires. Listen to `blur` there."],
    related: ["event-restore", "minimize", "is-minimized", "event-visibilitychange"],
  },
  {
    slug: "event-restore",
    name: "restore",
    kind: "event",
    summary: "Fires on `appWindow` when the main window comes back from minimized.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: 'appWindow.addEventListener("restore", (event: Event) => void)',
    notes: [HOST_EVENT, "It means only that the window left the minimized state; leaving maximized is `unmaximize`."],
    examples: [
      {
        title: "Clear the badge when the user restores the window",
        lang: "ts",
        code: `${IMPORT}

appWindow.addEventListener("restore", () => {
  void navigator.clearAppBadge();
});`,
      },
    ],
    limitations: ["On Linux under Wayland, it never fires. Listen to `focus` there."],
    related: ["event-minimize", "restore", "is-minimized"],
  },
  {
    slug: "event-move",
    name: "move",
    kind: "event",
    summary: "Fires on `appWindow` when the main window's position on the screen changes.",
    platforms: os(OK, PART, PART),
    syntax: 'appWindow.addEventListener("move", (event: Event) => void)',
    notes: [HOST_EVENT, "While the user drags the window, it fires once a frame, not once per pixel."],
    examples: [
      {
        title: "Save the window position after the user moves it",
        lang: "ts",
        code: `${IMPORT}

let timer = 0;
appWindow.addEventListener("move", () => {
  clearTimeout(timer);
  timer = setTimeout(() => {
    localStorage.setItem("window-bounds", JSON.stringify(appWindow.getBounds()));
  }, 500);
});`,
      },
    ],
    edgeCases: ["On Windows, minimizing moves the window far off-screen, which may fire it."],
    limitations: [
      "On macOS, the position is relative to the current screen, so a move to another screen can report a jump.",
      "On Linux under Wayland, it is not verified.",
    ],
    related: ["screen-x", "get-bounds"],
  },
  {
    slug: "event-fullscreenchange",
    name: "fullscreenchange (appWindow)",
    kind: "event",
    summary: "Fires on `appWindow` when the main window enters or leaves fullscreen, by any route.",
    platforms: all(OK),
    syntax: 'appWindow.addEventListener("fullscreenchange", (event: Event) => void)',
    notes: [
      HOST_EVENT,
      "It covers `setFullscreen()`, element fullscreen and the macOS green button. On macOS, AppKit’s did-enter or did-exit notification marks completion.",
      "It fires after `resize`, as the Fullscreen spec orders them.",
    ],
    examples: [
      {
        title: "Mark the page while the window is fullscreen",
        lang: "ts",
        code: `${IMPORT}

appWindow.addEventListener("fullscreenchange", () => {
  document.body.classList.toggle("fullscreen", appWindow.isFullscreen());
});`,
      },
    ],
    related: ["is-fullscreen", "set-fullscreen", "event-document-fullscreenchange"],
  },
  {
    slug: "event-document-fullscreenchange",
    name: "fullscreenchange (element)",
    kind: "event",
    summary: "Fires on an element when it enters or leaves fullscreen, and bubbles to the document.",
    platforms: all(OK),
    mdn: `${MDN}API/Element/fullscreenchange_event`,
    syntax: 'element.addEventListener("fullscreenchange", (event: Event) => void)',
    notes: [
      "It fires once the layout matches the new state. If the element has left the page, it fires on the document.",
      "It does not fire for `appWindow.setFullscreen()`.",
    ],
    examples: [
      {
        title: "Swap the fullscreen button label",
        lang: "ts",
        code: `export function syncButton(player: HTMLElement, button: HTMLButtonElement) {
  player.addEventListener("fullscreenchange", () => {
    button.textContent = document.fullscreenElement === player ? "Exit fullscreen" : "Fullscreen";
  });
}`,
      },
    ],
    related: ["request-fullscreen", "exit-fullscreen", "event-fullscreenchange"],
  },
  {
    slug: "event-fullscreenerror",
    name: "fullscreenerror",
    kind: "event",
    summary: "The event for a refused fullscreen request; it never fires.",
    platforms: all(NO),
    mdn: `${MDN}API/Element/fullscreenerror_event`,
    syntax: 'document.addEventListener("fullscreenerror", (event: Event) => void)',
    workaround: "Catch the rejection of the promise that `requestFullscreen()` returns.",
    related: ["request-fullscreen"],
  },
  {
    slug: "event-backdropchange",
    name: "backdropchange",
    kind: "event",
    summary: "Fires on `appWindow` when the window material, asked for or drawn, changes.",
    platforms: all(OK),
    syntax: 'appWindow.addEventListener("backdropchange", (event: Event) => void)',
    notes: [HOST_EVENT, "It fires when `backdrop` or `effectiveBackdrop` changes, as when macOS enters fullscreen."],
    examples: [
      {
        title: "Switch to an opaque theme when no material is drawn",
        lang: "ts",
        code: `${IMPORT}

const sync = () =>
  document.documentElement.classList.toggle("opaque", appWindow.effectiveBackdrop === "none");
appWindow.addEventListener("backdropchange", sync);
sync();`,
      },
    ],
    related: ["backdrop", "effective-backdrop"],
  },
  {
    slug: "event-visibilitychange",
    name: "visibilitychange",
    kind: "event",
    summary: "Fires on `document` when the main window becomes hidden or visible.",
    platforms: os(OK, PART, PART),
    platformSince: MAC_LINUX_0_3,
    mdn: `${MDN}API/Document/visibilitychange_event`,
    syntax: 'document.addEventListener("visibilitychange", (event: Event) => void)',
    notes: [
      "Hidden means hidden with `hide()` or minimized, and on Windows also on another virtual desktop or fully covered (also while focused), and while the system has turned the display off.",
      "It fires before `minimize` and `restore` in the same frame.",
    ],
    examples: [
      {
        title: "Stop polling while the window cannot be seen",
        lang: "ts",
        code: `let timer = 0;

export function poll(fetchStatus: () => void) {
  const sync = () => {
    clearInterval(timer);
    if (document.visibilityState === "visible") timer = setInterval(fetchStatus, 5000);
  };
  document.addEventListener("visibilitychange", sync);
  sync();
}`,
      },
    ],
    limitations: [
      "On macOS and Linux, covering the window or moving it to another desktop does not fire it.",
      "On Linux under Wayland, it never fires.",
    ],
    related: ["document-visibility-state", "event-minimize", "hide"],
  },
  {
    slug: "event-change",
    name: "change (MediaQueryList)",
    kind: "event",
    summary: "Fires on a `MediaQueryList` when its query starts or stops matching.",
    platforms: all(OK),
    mdn: `${MDN}API/MediaQueryList/change_event`,
    syntax: 'mediaQueryList.addEventListener("change", (event: MediaQueryListEvent) => void)',
    notes: [
      "Lists are re-evaluated after each window event and after a change of the CSS environment, such as the theme.",
      "It fires only when `matches` flips. `event.matches` and `event.media` are set.",
      "`onchange` and `addListener` work too.",
    ],
    examples: [
      {
        title: "Redraw a chart when the theme changes",
        lang: "ts",
        code: `export function followTheme(redraw: (dark: boolean) => void) {
  const dark = matchMedia("(prefers-color-scheme: dark)");
  redraw(dark.matches);
  dark.addEventListener("change", (e) => redraw(e.matches));
}`,
        note: "It fires also when the app sets `appWindow.theme`.",
      },
    ],
    related: ["match-media", "theme"],
    aliases: ["MediaQueryListEvent"],
  },
]);

const CREATION_ONLY = "It is read once when the window opens; a change needs a restart.";
const OVERLAY_LIMITS = [
  "On macOS the height repositions the native traffic lights. Custom button colours and host-drawn controls are unavailable.",
];
const HEX_ONLY =
  "Only hex colors are accepted: `#rgb`, `#rrggbb` or `#rrggbbaa`. Another form, such as a color name or `rgb()`, is refused with an advisory.";

const startup = section("Startup configuration", [
  {
    slug: "define-window",
    name: "defineWindow()",
    kind: "function",
    summary:
      "Sets the main window's run-time options from JS: title, backdrop, theme, always on top and the title bar overlay.",
    platforms: os(OK, PART, PART),
    syntax:
      'import { defineWindow } from "@skathi-native/runtime";\n\ndefineWindow(options: { title?: string; backdrop?: string; theme?: string; alwaysOnTop?: boolean; visibleOnAllWorkspaces?: boolean; opacity?: number; titleBarHeight?: number; titleBarOverlay?: boolean | object }): void',
    params: [
      { name: "options.title", type: "string", description: "Sets `document.title`.", optional: true },
      { name: "options.backdrop", type: "string", description: "Sets `appWindow.backdrop`.", optional: true },
      {
        name: "options.theme",
        type: "string",
        description: "Sets `appWindow.theme`.",
        optional: true,
      },
      { name: "options.alwaysOnTop", type: "boolean", description: "Sets `appWindow.alwaysOnTop`.", optional: true },
      {
        name: "options.titleBarHeight",
        type: "number",
        description: "Sets the height of the title bar area and keeps the overlay as it is.",
        optional: true,
      },
      {
        name: "options.titleBarOverlay",
        type: "boolean | { height?, color?, symbolColor? }",
        description: "Calls `appWindow.setTitleBarOverlay()`.",
        optional: true,
      },
    ],
    returns: "`undefined`.",
    notes: [
      "It runs when the app's code runs, after the window exists. Each key goes through the same setter as the matching `appWindow` property, so each has that property's platform support.",
      "`frame` and `titleBarStyle` print an advisory that they need a different window style. `width`, `height`, `minWidth`, `minHeight`, `resizable`, `minimizable`, `maximizable`, `titleBarFallback` and `focus` print an advisory that they are creation-time only. Any other key prints an advisory that lists the accepted keys.",
      "`null` or a value that is not an object does nothing.",
    ],
    examples: [
      {
        title: "Set the window up from the app's entry module",
        lang: "ts",
        code: `import { createApp } from "vue";
import { defineWindow } from "@skathi-native/runtime";
import App from "./App.vue";

defineWindow({
  title: "Ledger",
  backdrop: "mica-alt",
  titleBarOverlay: { height: 40, color: "#00000000", symbolColor: "#1f1f1f" },
});

createApp(App).mount("#app");`,
      },
    ],
    limitations: [
      "On macOS and Linux, `backdrop` and `theme` fall back as described under `appWindow.backdrop` and `appWindow.theme`. On macOS, `titleBarOverlay` is ignored.",
      "On Linux under Wayland, `alwaysOnTop` prints an advisory and does nothing.",
    ],
    related: ["config-window", "app-window", "set-title-bar-overlay", "backdrop"],
  },
  {
    slug: "config-window",
    name: "window",
    kind: "option",
    summary: "The block of `sk.config.json` that describes the main window as it opens.",
    platforms: all(OK),
    syntax:
      '{\n  "window": {\n    "title": string,\n    "width": number,\n    "height": number,\n    "minWidth": number,\n    "minHeight": number,\n    "x": number,\n    "y": number,\n    "center": boolean,\n    "monitor": "primary" | "cursor" | number,\n    "titleBarStyle": "hidden" | "default" | "hiddenInset",\n    "titleBarHeight": number,\n    "titleBarFallback": boolean,\n    "titleBarOverlay": boolean | { "height"?: number, "color"?: string, "symbolColor"?: string },\n    "backdrop": string,\n    "resizable": boolean,\n    "minimizable": boolean,\n    "alwaysOnTop": boolean,\n    "visibleOnAllWorkspaces": boolean,\n    "opacity": number,\n    "focus": "raise",\n    "devtools": boolean\n  }\n}',
    notes: [
      CREATION_ONLY,
      "The window opens centered on the primary display, unless `x`, `y` and `monitor` place it elsewhere. No key sets a maximized, fullscreen or hidden start, or the theme.",
      "An unknown key is ignored without a warning. A value of the wrong type keeps the default.",
      "The environment variable `SK_WINDOW=1280x800` overrides `width` and `height`.",
      "Changing the window later is done through `appWindow` and `defineWindow()`.",
    ],
    examples: [
      {
        title: "A note-taking app with its own title bar",
        lang: "ts",
        code: `{
  "name": "notes",
  "window": {
    "title": "Notes",
    "width": 960,
    "height": 640,
    "minWidth": 480,
    "minHeight": 320,
    "titleBarStyle": "hidden",
    "titleBarFallback": false,
    "backdrop": "mica"
  }
}`,
      },
    ],
    edgeCases: [
      "Nothing saves the window's size or position between launches. Store `appWindow.getBounds()` and restore it with `setBounds()`.",
    ],
    related: ["define-window", "config-window-title-bar-style", "config-window-width", "app-window"],
    aliases: ["sk.config.json", "window options"],
  },
  {
    slug: "config-window-low-latency-surface",
    name: "window.lowLatencySurface",
    kind: "option",
    summary: "Lets a macOS player window present frames on demand.",
    since: "0.6.0",
    platforms: os(NO, OK, NO),
    syntax: '{ "window": { "lowLatencySurface": true } }',
    notes: [
      CREATION_ONLY,
      "Defaults to false. Isolated surface frames wake the window immediately after a pause. The display link paces continuous playback and UI animation. Display sync stays enabled.",
    ],
    related: ["sk-window-low-latency-surface"],
  },
  {
    slug: "config-window-title",
    name: "window.title",
    kind: "option",
    summary: "Sets the main window's title when the app starts.",
    platforms: all(OK),
    syntax: '{ "window": { "title": string } }',
    params: [
      {
        name: "title",
        type: "string",
        description: "The OS caption and the initial `document.title`.",
        default: '`"Skathi"`',
        optional: true,
      },
    ],
    notes: [
      "It also names the app in permission and crash dialogs, and is the default `productName`.",
      "Change it at run time with `appWindow.title` or `document.title`.",
    ],
    examples: [
      {
        title: "Name the window",
        lang: "ts",
        code: `{
  "window": { "title": "Ledger" }
}`,
      },
    ],
    related: ["title", "document-title", "config-window-title-bar-fallback"],
  },
  {
    slug: "config-window-width",
    name: "window.width",
    kind: "option",
    summary: "Sets the main window's width when it opens, in CSS px.",
    platforms: all(OK),
    syntax: '{ "window": { "width": number } }',
    params: [
      { name: "width", type: "number > 0", description: "Initial width in CSS px.", default: "`640`", optional: true },
    ],
    notes: [CREATION_ONLY, "`SK_WINDOW=WxH` in the environment overrides it."],
    examples: [
      {
        title: "Open a wide editor window",
        lang: "ts",
        code: `{
  "window": { "width": 1280, "height": 800 }
}`,
      },
    ],
    related: ["config-window-height", "config-window-min-width", "set-content-bounds"],
  },
  {
    slug: "config-window-height",
    name: "window.height",
    kind: "option",
    summary: "Sets the main window's height when it opens, in CSS px.",
    platforms: all(OK),
    syntax: '{ "window": { "height": number } }',
    params: [
      {
        name: "height",
        type: "number > 0",
        description: "Initial height in CSS px.",
        default: "`420`",
        optional: true,
      },
    ],
    notes: [CREATION_ONLY, "`SK_WINDOW=WxH` in the environment overrides it."],
    examples: [
      {
        title: "A tall, narrow chat window",
        lang: "ts",
        code: `{
  "window": { "width": 420, "height": 760 }
}`,
      },
    ],
    related: ["config-window-width", "config-window-min-height"],
  },
  {
    slug: "config-window-visible-on-all-workspaces",
    name: "window.visibleOnAllWorkspaces",
    kind: "option",
    summary: "Sets initial workspace membership for the main window.",
    since: "0.6.0",
    platforms: os(NO, OK, PART),
    syntax: '{ "window": { "alwaysOnTop": true, "visibleOnAllWorkspaces": true } }',
    related: ["visible-on-all-workspaces", "always-on-top"],
  },
  {
    slug: "config-window-opacity",
    name: "window.opacity",
    kind: "option",
    summary: "Sets native window opacity before the main window is shown.",
    since: "0.6.0",
    platforms: os(OK, OK, PART),
    syntax: '{ "window": { "opacity": 0.85 } }',
    related: ["opacity", "show", "hide"],
  },
  {
    slug: "config-window-min-width",
    name: "window.minWidth",
    kind: "option",
    summary: "Sets the narrowest the user can make the main window, in CSS px.",
    platforms: all(OK),
    syntax: '{ "window": { "minWidth": number } }',
    params: [{ name: "minWidth", type: "number >= 0", description: "Minimum width in CSS px.", optional: true }],
    notes: [
      CREATION_ONLY,
      "It sets the initial width. setMinSize() changes or clears the limit at runtime.",
      "On Windows, a value above 500 prints an advisory: the Windows 11 snap layouts reject such a window.",
    ],
    examples: [
      {
        title: "Keep a two-pane layout from collapsing",
        lang: "ts",
        code: `{
  "window": { "minWidth": 480, "minHeight": 320 }
}`,
      },
    ],
    edgeCases: ["Use appWindow.setMaxSize() for a live maximum."],
    related: ["config-window-min-height", "sk-window-min-width"],
  },
  {
    slug: "config-window-min-height",
    name: "window.minHeight",
    kind: "option",
    summary: "Sets the shortest the user can make the main window, in CSS px.",
    platforms: all(OK),
    syntax: '{ "window": { "minHeight": number } }',
    params: [{ name: "minHeight", type: "number >= 0", description: "Minimum height in CSS px.", optional: true }],
    notes: ["It sets the initial minimum height. setMinSize() changes or clears the limit at runtime."],
    examples: [
      {
        title: "Leave room for a toolbar and a status bar",
        lang: "ts",
        code: `{
  "window": { "minWidth": 360, "minHeight": 240 }
}`,
      },
    ],
    related: ["config-window-min-width", "sk-window-min-height"],
  },
  {
    slug: "config-window-x",
    name: "window.x",
    kind: "option",
    summary: "Places the main window when it is created: CSS px from the left edge of its monitor's work area.",
    platforms: os(OK, OK, PART),
    since: "0.4.0",
    syntax: '{ "window": { "x": number } }',
    params: [
      {
        name: "x",
        type: "number",
        description: "CSS px from the left edge of the work area to the window.",
        default: "centred",
        optional: true,
      },
    ],
    notes: [
      CREATION_ONLY,
      "The window is created at that place, so it never shows at a default one first.",
      "The work area is the monitor minus the taskbar or the Dock. `window.monitor` picks the monitor; the default is the primary one.",
      'Without it the window is centred horizontally: `{ "y": 0 }` opens at the top, centred.',
      'The window is kept inside the work area. A value past an edge puts the window against that edge, and a window larger than the work area is shrunk to fit it. Under `titleBarStyle: "default"` the OS caption stays on screen too.',
      "On Windows, the number is in the target monitor's own scale.",
      "With `center: true` as well, the window is centred and `sk build` warns.",
    ],
    examples: [
      {
        title: "Open a narrow side window at the top-left of the screen",
        lang: "ts",
        code: `{
  "window": { "width": 380, "height": 720, "x": 16, "y": 16 }
}`,
      },
    ],
    limitations: ["On Linux under Wayland, the compositor places the window and `x` is ignored."],
    related: ["config-window-y", "config-window-monitor", "config-window-center", "set-bounds"],
    aliases: ["position", "start position", "initial position"],
  },
  {
    slug: "config-window-y",
    name: "window.y",
    kind: "option",
    summary: "Places the main window when it is created: CSS px from the top edge of its monitor's work area.",
    platforms: os(OK, OK, PART),
    since: "0.4.0",
    syntax: '{ "window": { "y": number } }',
    params: [
      {
        name: "y",
        type: "number",
        description: "CSS px from the top edge of the work area to the window.",
        default: "centred",
        optional: true,
      },
    ],
    notes: [
      CREATION_ONLY,
      "Without it the window is centred vertically. It works as `window.x` does, on the other axis.",
      'Under `titleBarStyle: "default"`, it places the top of the OS caption, which stays inside the work area.',
    ],
    examples: [
      {
        title: "Open at the top of the screen, centred horizontally",
        lang: "ts",
        code: `{
  "window": { "y": 0 }
}`,
      },
    ],
    limitations: ["On Linux under Wayland, the compositor places the window and `y` is ignored."],
    related: ["config-window-x", "config-window-monitor", "set-bounds"],
  },
  {
    slug: "config-window-center",
    name: "window.center",
    kind: "option",
    summary: "Opens the main window centred on its monitor's work area.",
    platforms: os(OK, OK, PART),
    since: "0.4.0",
    syntax: '{ "window": { "center": boolean } }',
    params: [
      {
        name: "center",
        type: "boolean",
        description: "`true` centres the window, whatever `x` and `y` say.",
        default: "`false`",
        optional: true,
      },
    ],
    notes: [
      CREATION_ONLY,
      "Without `x` and `y` the window is centred anyway; `center: true` says so explicitly.",
      "With `x` or `y` as well, the window is centred, the host prints an advisory and `sk build` warns.",
    ],
    examples: [
      {
        title: "Centre the window on the monitor under the pointer",
        lang: "ts",
        code: `{
  "window": { "center": true, "monitor": "cursor" }
}`,
      },
    ],
    limitations: ["On Linux under Wayland, the compositor places the window."],
    related: ["config-window-x", "config-window-monitor", "center"],
  },
  {
    slug: "config-window-monitor",
    name: "window.monitor",
    kind: "option",
    summary: "Picks the monitor the main window opens on.",
    platforms: os(OK, PART, PART),
    since: "0.4.0",
    syntax: '{ "window": { "monitor": "primary" | "cursor" | number } }',
    params: [
      {
        name: "monitor",
        type: '"primary" | "cursor" | number',
        description:
          '`"primary"`: the primary monitor. `"cursor"`: the monitor under the mouse pointer. A number: the monitor at that index, from `0`, in the OS\'s own order.',
        default: '`"primary"`',
        optional: true,
      },
    ],
    notes: [
      CREATION_ONLY,
      "`x` and `y` are measured from this monitor's work area; without them the window is centred on it.",
      "An index past the last monitor opens the window on the primary monitor and prints an advisory.",
      "The indices are in the same order as `<SkWindow :monitor>`.",
    ],
    examples: [
      {
        title: "Open where the user is looking",
        lang: "ts",
        code: `{
  "window": { "monitor": "cursor" }
}`,
      },
      {
        title: "Open on the second monitor, at its top-right",
        lang: "ts",
        code: `{
  "window": { "monitor": 1, "x": 100000, "y": 0 }
}`,
      },
    ],
    limitations: [
      'On macOS and Linux, `"cursor"` opens on the primary monitor with an advisory.',
      "On Linux under Wayland, the compositor picks the monitor.",
    ],
    related: ["config-window-x", "config-window-center", "sk-window-monitor"],
    aliases: ["display", "screen"],
  },
  {
    slug: "config-window-frame",
    name: "window.frame",
    kind: "option",
    summary: "The Electron option for a frameless window; `false` only prints an advisory.",
    platforms: all(NO),
    syntax: '{ "window": { "frame": boolean } }',
    notes: [
      "A frameless window would lose its shadow, its animations and its resize edges, so Skathi keeps the frame.",
      'It does not change `titleBarStyle`: with `"default"`, the OS title bar stays.',
    ],
    workaround:
      'Keep the default `titleBarStyle: "hidden"`, which hides the OS title bar and keeps the frame, and draw the title bar in the app.',
    related: ["config-window-title-bar-style"],
    aliases: ["frameless"],
  },
  {
    slug: "config-window-title-bar-style",
    name: "window.titleBarStyle",
    kind: "option",
    summary: "Chooses whether the app draws its own title bar or the OS draws one.",
    platforms: all(OK),
    syntax: '{ "window": { "titleBarStyle": "hidden" | "default" | "hiddenInset" } }',
    values: [
      v("hidden", OK, "The default. The app draws the title bar. On macOS the traffic lights stay at its left."),
      v("default", OK, "The OS draws the title bar and its buttons."),
      v("hiddenInset", PART, 'Treated as `"hidden"` on every OS, macOS included, with an advisory.'),
    ],
    notes: [
      CREATION_ONLY,
      '`"hidden"`: `display-mode` is `window-controls-overlay`, and `app-region` and `--window-control` are live.',
      '`"default"`: `display-mode` is `standalone`, `titleBarFallback` is off, and `app-region`, `--window-control` and `setTitleBarOverlay()` do nothing, with advisories.',
      'An unknown value prints an advisory and keeps `"hidden"`.',
      'On macOS, with `"hidden"`, the traffic lights are centered in `titleBarHeight`, and the window drags by the app\'s `app-region` areas, not by the strip where the system title bar sits.',
      'On Linux, with `"hidden"`, the host moves, resizes, maximizes and closes the window from the app\'s regions itself.',
    ],
    examples: [
      {
        title: "Use the OS title bar",
        lang: "ts",
        code: `{
  "window": { "title": "Settings", "titleBarStyle": "default" }
}`,
      },
    ],
    related: ["config-window-title-bar-height", "app-region", "window-control", "display-mode", "chrome"],
    aliases: ["titlebar", "custom title bar", "frameless"],
  },
  {
    slug: "config-window-title-bar-height",
    name: "window.titleBarHeight",
    kind: "option",
    summary: "Sets the height of the title bar area the app draws, in CSS px.",
    platforms: os(OK, PART, OK),
    syntax: '{ "window": { "titleBarHeight": number } }',
    params: [
      {
        name: "titleBarHeight",
        type: "number >= 0",
        description: "Height in CSS px.",
        default: "`32`",
        optional: true,
      },
    ],
    notes: [
      "It is the height of `env(titlebar-area-height)`, of `getTitlebarAreaRect()`, of the caption buttons from `titleBarOverlay` and of the host's temporary title bar strip.",
      "Change it at run time with `defineWindow({ titleBarHeight })` or `appWindow.setTitleBarOverlay({ height })`.",
    ],
    examples: [
      {
        title: "A taller title bar that holds tabs",
        lang: "ts",
        code: `{
  "window": { "titleBarHeight": 44, "titleBarOverlay": true }
}`,
      },
    ],
    limitations: [
      "On macOS, automatic traffic light placement follows height changes. An explicit position takes precedence.",
    ],
    related: ["env-titlebar-area", "config-window-title-bar-overlay-height", "get-titlebar-area-rect"],
  },
  {
    slug: "config-window-title-bar-fallback",
    name: "window.titleBarFallback",
    kind: "option",
    summary: "Draws a host title bar strip until the app paints its own drag region.",
    platforms: all(OK),
    syntax: '{ "window": { "titleBarFallback": boolean } }',
    params: [
      {
        name: "titleBarFallback",
        type: "boolean",
        description: "Whether to draw the strip.",
        default: "`true`",
        optional: true,
      },
    ],
    notes: [
      "With `true`, the host draws a strip above the app that shows the current `document.title` and drags the window. It is `titleBarHeight` tall, 32 px by default. It goes away for good at the first frame in which the app paints an `app-region: drag` area.",
      "While the strip is shown, the Window Controls Overlay is not visible, `env(titlebar-area-*)` is undefined, `innerHeight` excludes the strip and `appWindow.chrome.contentInset.top` is its height.",
      "Its removal fires `geometrychange`.",
      'It is ignored, with an advisory, under `titleBarStyle: "default"`.',
    ],
    examples: [
      {
        title: "Turn the strip off for an app that always draws its title bar",
        lang: "ts",
        code: `{
  "window": { "titleBarStyle": "hidden", "titleBarFallback": false }
}`,
        note: "Then `env(titlebar-area-height)` has its value from the first frame.",
      },
    ],
    related: ["config-window-title-bar-style", "app-region", "chrome", "event-geometrychange"],
  },
  {
    slug: "config-window-title-bar-overlay",
    name: "window.titleBarOverlay",
    kind: "option",
    summary: "Draws minimize, maximize and close buttons over the top-right of the app's own title bar.",
    platforms: os(OK, NO, OK),
    syntax:
      '{ "window": { "titleBarOverlay": boolean | { "height"?: number, "color"?: string, "symbolColor"?: string } } }',
    params: [
      { name: "height", type: "number", description: "Sets `titleBarHeight`.", optional: true },
      {
        name: "color",
        type: "string",
        description: "Background of the buttons, in hex.",
        default: "transparent",
        optional: true,
      },
      {
        name: "symbolColor",
        type: "string",
        description: "Color of the glyphs, in hex.",
        default: "`#000000e0` in the light theme, `#ffffffe0` in the dark one",
        optional: true,
      },
    ],
    notes: [
      "`true` or an object installs three 46 px buttons. `false` or no value installs none.",
      "On Windows, the OS performs the actions, and hovering maximize opens the snap layouts flyout. On Linux, the host performs them.",
      "The buttons' width is taken out of `env(titlebar-area-width)` and of `getTitlebarAreaRect().width`.",
      "They are hidden in fullscreen and while minimized. The default glyph color and the hover colors of minimize and maximize follow the window's light or dark theme; close turns red.",
      'Under `titleBarStyle: "default"`, it is dropped.',
      "Like Electron's `titleBarOverlay`.",
    ],
    examples: [
      {
        title: "Native caption buttons on a dark title bar",
        lang: "ts",
        code: `{
  "window": {
    "titleBarStyle": "hidden",
    "titleBarOverlay": { "height": 36, "color": "#1f1f1f", "symbolColor": "#ffffff" }
  }
}`,
      },
    ],
    limitations: OVERLAY_LIMITS,
    related: [
      "set-title-bar-overlay",
      "config-window-title-bar-overlay-color",
      "config-window-title-bar-overlay-symbol-color",
      "window-control",
    ],
    aliases: ["caption buttons", "window controls"],
  },
  {
    slug: "config-window-title-bar-overlay-height",
    name: "window.titleBarOverlay.height",
    kind: "option",
    summary: "Sets the height of the overlay buttons and of the title bar area.",
    platforms: os(OK, NO, OK),
    syntax: '{ "window": { "titleBarOverlay": { "height": number } } }',
    params: [
      {
        name: "height",
        type: "number >= 0",
        description: "Height in CSS px.",
        default: "`titleBarHeight`, `32`",
        optional: true,
      },
    ],
    notes: ["It sets `titleBarHeight`, and wins when both are given."],
    examples: [
      {
        title: "Match the buttons to a 40 px toolbar",
        lang: "ts",
        code: `{
  "window": { "titleBarOverlay": { "height": 40 } }
}`,
      },
    ],
    limitations: OVERLAY_LIMITS.slice(0, 1),
    related: ["config-window-title-bar-overlay", "config-window-title-bar-height"],
  },
  {
    slug: "config-window-title-bar-overlay-color",
    name: "window.titleBarOverlay.color",
    kind: "option",
    summary: "Sets the background color of the overlay buttons.",
    platforms: os(OK, NO, OK),
    syntax: '{ "window": { "titleBarOverlay": { "color": "#rgb" | "#rrggbb" | "#rrggbbaa" } } }',
    params: [{ name: "color", type: "string", description: "A hex color.", default: "transparent", optional: true }],
    notes: [HEX_ONLY],
    examples: [
      {
        title: "Buttons on a branded title bar",
        lang: "ts",
        code: `{
  "window": { "titleBarOverlay": { "color": "#0b5cad", "symbolColor": "#ffffff" } }
}`,
      },
    ],
    limitations: OVERLAY_LIMITS.slice(0, 1),
    related: ["config-window-title-bar-overlay", "config-window-title-bar-overlay-symbol-color"],
  },
  {
    slug: "config-window-title-bar-overlay-symbol-color",
    name: "window.titleBarOverlay.symbolColor",
    kind: "option",
    summary: "Sets the color of the glyphs on the overlay buttons.",
    platforms: os(OK, NO, OK),
    syntax: '{ "window": { "titleBarOverlay": { "symbolColor": "#rgb" | "#rrggbb" | "#rrggbbaa" } } }',
    params: [
      {
        name: "symbolColor",
        type: "string",
        description: "A hex color.",
        default: "`#000000e0` in the light theme, `#ffffffe0` in the dark one",
        optional: true,
      },
    ],
    notes: [HEX_ONLY, "The default follows the window's theme, `appWindow.theme` included."],
    examples: [
      {
        title: "Dark glyphs on a light title bar",
        lang: "ts",
        code: `{
  "window": { "titleBarOverlay": { "color": "#f3f3f3", "symbolColor": "#1b1b1b" } }
}`,
      },
    ],
    limitations: OVERLAY_LIMITS.slice(0, 1),
    related: ["config-window-title-bar-overlay", "config-window-title-bar-overlay-color"],
  },
  {
    slug: "config-window-backdrop",
    name: "window.backdrop",
    kind: "option",
    summary: "Sets the material behind the main window's transparent content when the app starts.",
    platforms: os(OK, PART, PART),
    syntax:
      '{ "window": { "backdrop": "mica" | "mica-alt" | "tabbed" | "acrylic" | "blur" | "transparent" | "none" | "opaque" } }',
    values: [
      v("mica", OK, "The default."),
      v("mica-alt", OK, "`tabbed` is a synonym."),
      v("acrylic", OK, "`blur` is a synonym."),
      v("transparent", PART, "On Linux it depends on the compositor."),
      v("none", OK, "`opaque` is a synonym."),
    ],
    notes: [
      "On Windows before build 22621, `mica` and `mica-alt` become `acrylic`, with an advisory.",
      "An unknown value prints an advisory and keeps `mica`.",
      "It is also the default backdrop of every `<SkWindow>`, even after the app changes `appWindow.backdrop`.",
      "Change it at run time with `appWindow.backdrop`.",
    ],
    examples: [
      {
        title: "A blurred, translucent launcher",
        lang: "ts",
        code: `{
  "window": { "width": 640, "height": 72, "backdrop": "acrylic", "resizable": false }
}`,
      },
    ],
    limitations: [
      "On macOS, the three materials draw the same behind-window blur.",
      "On Linux, the three materials draw an opaque fill.",
    ],
    related: ["backdrop", "sk-window-backdrop"],
  },
  {
    slug: "config-window-resizable",
    name: "window.resizable",
    kind: "option",
    summary: "Sets whether the user can resize the main window.",
    platforms: all(OK),
    platformSince: { linux: "0.3.0" },
    syntax: '{ "window": { "resizable": boolean } }',
    params: [
      {
        name: "resizable",
        type: "boolean",
        description: "`false` removes the resize edges.",
        default: "`true`",
        optional: true,
      },
    ],
    notes: [
      CREATION_ONLY,
      "`false` also removes maximize. On Windows, the maximize button area then acts as caption, and no snap layouts flyout appears.",
      "On Linux, `false` also fixes the size for code: `setBounds()` and `resizeTo()` cannot change it.",
    ],
    examples: [
      {
        title: "A fixed-size calculator",
        lang: "ts",
        code: `{
  "window": { "width": 320, "height": 480, "resizable": false }
}`,
      },
    ],
    related: ["config-window-minimizable", "config-window-maximizable", "sk-window-resizable"],
  },
  {
    slug: "config-window-minimizable",
    name: "window.minimizable",
    kind: "option",
    summary: "Sets whether the main window has a minimize button.",
    platforms: all(OK),
    platformSince: { linux: "0.3.0" },
    syntax: '{ "window": { "minimizable": boolean } }',
    params: [
      {
        name: "minimizable",
        type: "boolean",
        description: "`false` removes minimize.",
        default: "`true`",
        optional: true,
      },
    ],
    notes: [
      CREATION_ONLY,
      "A `--window-control: minimize` area then does nothing. `appWindow.minimize()` still works.",
    ],
    examples: [
      {
        title: "A setup wizard that cannot be minimized",
        lang: "ts",
        code: `{
  "window": { "title": "Setup", "minimizable": false, "resizable": false }
}`,
      },
    ],
    related: ["config-window-resizable", "minimize"],
  },
  {
    slug: "config-window-maximizable",
    name: "window.maximizable",
    kind: "option",
    summary: "Sets whether the user can maximize the main window.",
    platforms: all(OK),
    since: "0.3.0",
    syntax: '{ "window": { "maximizable": boolean } }',
    params: [
      {
        name: "maximizable",
        type: "boolean",
        description: "`false` removes maximize.",
        default: "`true`",
        optional: true,
      },
    ],
    notes: [
      CREATION_ONLY,
      "With `false`, the maximize button, a double-click on a drag area and a `--window-control: maximize` area no longer maximize the window.",
      "On macOS, the zoom button is disabled.",
      "`resizable: false` removes maximize as well.",
    ],
    examples: [
      {
        title: "A preferences window that cannot be maximized",
        lang: "ts",
        code: `{
  "window": { "title": "Preferences", "maximizable": false }
}`,
      },
    ],
    related: ["config-window-resizable", "config-window-minimizable", "maximize"],
  },
  {
    slug: "config-window-always-on-top",
    name: "window.alwaysOnTop",
    kind: "option",
    summary: "Starts the main window above other windows.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: '{ "window": { "alwaysOnTop": boolean } }',
    params: [
      {
        name: "alwaysOnTop",
        type: "boolean",
        description: "Whether the window starts topmost.",
        default: "`false`",
        optional: true,
      },
    ],
    notes: ["Change it at run time with `appWindow.alwaysOnTop`."],
    examples: [
      {
        title: "A small timer that floats above everything",
        lang: "ts",
        code: `{
  "window": { "title": "Timer", "width": 240, "height": 120, "alwaysOnTop": true }
}`,
      },
    ],
    limitations: ["On Linux under Wayland, it prints an advisory and does nothing."],
    related: ["always-on-top"],
  },
  {
    slug: "config-window-focus",
    name: "window.focus",
    kind: "option",
    summary: "Lets `window.focus()` raise the main window instead of only asking for attention.",
    platforms: os(OK, OK, PART),
    syntax: '{ "window": { "focus": "raise" } }',
    params: [{ name: "focus", type: '"raise"', description: "The only accepted value.", optional: true }],
    notes: [
      "`SK_WINDOW_FOCUS=raise` in the environment does the same.",
      "Any other value is ignored without a warning.",
      "It does not decide whether the window comes to the front when the app starts.",
    ],
    examples: [
      {
        title: "Let a timer app jump to the front when time is up",
        lang: "ts",
        code: `{
  "window": { "focus": "raise" }
}`,
        note: "Then `window.focus()` in the app activates and raises the window.",
      },
    ],
    limitations: ["On Linux under Wayland, the compositor may refuse to raise the window."],
    related: ["window-focus", "show"],
  },
  {
    slug: "config-window-devtools",
    name: "window.devtools",
    kind: "option",
    summary: "Turns the devtools connection off or on for this app, overriding the top-level `devtools` key.",
    platforms: all(OK),
    syntax: '{ "window": { "devtools": boolean } }',
    params: [
      {
        name: "devtools",
        type: "boolean",
        description: "`false` starts no devtools listener and no F12.",
        optional: true,
      },
    ],
    notes: [
      "It sits in the `window` block because it must be read before the app's code runs.",
      "`SK_DEVTOOLS=1` in the environment overrides it.",
    ],
    examples: [
      {
        title: "Ship without devtools",
        lang: "ts",
        code: `{
  "window": { "devtools": false }
}`,
      },
    ],
    related: ["config-window"],
  },
  {
    slug: "config-single-instance",
    name: "singleInstance",
    kind: "option",
    summary: "Keeps one running copy of the app: a second launch hands its arguments to the first and exits.",
    platforms: all(OK),
    platformSince: { macos: "0.3.0" },
    syntax: '{ "singleInstance": boolean }',
    params: [
      {
        name: "singleInstance",
        type: "boolean",
        description: "Top-level key of `sk.config.json`.",
        default: "`false`",
        optional: true,
      },
    ],
    notes: [
      "The lock is keyed by the app's `id`.",
      'A second launch forwards its arguments, working directory and URL, exits with code 0 and opens no window. The first instance receives them as a launch event with `source: "second-instance"`, or `"open-url"` when a link started it.',
      "If the first instance does not answer within 5 s, the second launch exits with code 2. If it is closing, the second launch waits up to 6 s and starts afresh.",
      "`SK_SINGLE_INSTANCE=0` in the environment turns it off.",
    ],
    examples: [
      {
        title: "Open files in the running editor",
        lang: "ts",
        code: `{
  "id": "com.example.writer",
  "singleInstance": true,
  "raiseOnSecondInstance": true
}`,
      },
    ],
    related: ["config-raise-on-second-instance", "config-id"],
  },
  {
    slug: "config-raise-on-second-instance",
    name: "raiseOnSecondInstance",
    kind: "option",
    summary: "Brings the running app's window to the front when a second launch is forwarded to it.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: '{ "raiseOnSecondInstance": boolean }',
    params: [
      {
        name: "raiseOnSecondInstance",
        type: "boolean",
        description: "Top-level key of `sk.config.json`.",
        default: "`true`",
        optional: true,
      },
    ],
    notes: ["`SK_LAUNCH_RAISE=off` in the environment turns it off."],
    examples: [
      {
        title: "Keep a background app in the tray on a second launch",
        lang: "ts",
        code: `{
  "singleInstance": true,
  "raiseOnSecondInstance": false
}`,
      },
    ],
    limitations: [
      "On Linux under Wayland, the compositor may only flag the window for attention instead of raising it.",
    ],
    related: ["config-single-instance", "show"],
  },
]);

const MAC_TITLEBAR_X =
  "On macOS, the area starts past the traffic lights. The default inset is 78 px and follows a custom horizontal position.";

const titleBar = section("Title bar and chrome", [
  {
    slug: "chrome",
    name: "appWindow.chrome",
    kind: "property",
    summary:
      "Describes the main window's title bar setup: who draws it, its height, the overlay buttons and the client area.",
    platforms: all(OK),
    syntax:
      'readonly appWindow.chrome: {\n  titleBarStyle: "default" | "hidden";\n  displayMode: "standalone" | "window-controls-overlay" | "fullscreen";\n  titleBarHeight: number;\n  fallbackTitleBar: boolean;\n  overlayVisible: boolean;\n  hostControls: boolean;\n  client: { width: number; height: number };\n  contentInset: { left: number; top: number; right: number; bottom: number };\n}',
    params: [
      { name: "titleBarStyle", type: "string", description: "The `titleBarStyle` the window opened with." },
      { name: "displayMode", type: "string", description: "The value `@media (display-mode)` matches." },
      {
        name: "titleBarHeight",
        type: "number",
        description: "The height of the title bar area, `0` whenever the area is not visible.",
      },
      { name: "fallbackTitleBar", type: "boolean", description: "Whether the host's temporary strip is shown." },
      { name: "overlayVisible", type: "boolean", description: "Equals `navigator.windowControlsOverlay.visible`." },
      {
        name: "hostControls",
        type: "boolean",
        description:
          "Whether the host draws the caption buttons, so the app must not draw its own: the `titleBarOverlay` buttons, or on macOS the traffic lights while they are shown.",
      },
      { name: "client", type: "object", description: "The client area in CSS px." },
      {
        name: "contentInset",
        type: "object",
        description:
          "How far host chrome, such as the temporary strip, insets the app: `innerWidth` is `client.width` minus the insets.",
      },
    ],
    notes: ["Each read returns a new object; compare its fields, not the object.", "It is reactive."],
    examples: [
      {
        title: "Draw window buttons only when the host does not",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}
</script>

<template>
  <header class="titlebar">
    <span class="title">{{ appWindow.title }}</span>
    <div v-if="!appWindow.chrome.hostControls" class="controls">
      <button class="min" />
      <button class="max" />
      <button class="close" />
    </div>
  </header>
</template>`,
      },
    ],
    edgeCases: [
      "`titleBarHeight` reads `0` in fullscreen, while minimized, with OS chrome, and while the temporary strip is shown. It is not the configured height.",
    ],
    related: ["config-window-title-bar-style", "window-controls-overlay", "set-title-bar-overlay", "display-mode"],
  },
  {
    slug: "set-title-bar-overlay",
    name: "appWindow.setTitleBarOverlay()",
    kind: "method",
    summary:
      "Installs, restyles or removes the minimize, maximize and close buttons the host draws over the title bar.",
    platforms: os(OK, PART, OK),
    syntax:
      "appWindow.setTitleBarOverlay(overlay: boolean | null | { height?: number; color?: string; symbolColor?: string }): void",
    params: [
      {
        name: "overlay",
        type: "boolean | null | object",
        description:
          "`false` or `null` removes the buttons. `true` installs them with the default colors. An object installs or updates them.",
      },
      { name: "overlay.height", type: "number", description: "Sets the title bar height.", optional: true },
      { name: "overlay.color", type: "string", description: "Background of the buttons, in hex.", optional: true },
      { name: "overlay.symbolColor", type: "string", description: "Color of the glyphs, in hex.", optional: true },
    ],
    returns: "`undefined`.",
    notes: [
      "It applies at once; read `appWindow.chrome.hostControls` right after.",
      "`true` resets custom colors to the defaults.",
      "A color in another form than hex prints an advisory and is ignored. A number or a string as the argument is ignored without a warning.",
      "`geometrychange` fires when the buttons appear or disappear.",
      "Like Electron's `win.setTitleBarOverlay()`.",
    ],
    examples: [
      {
        title: "Recolor the caption buttons with the theme",
        lang: "ts",
        code: `${IMPORT}

const dark = matchMedia("(prefers-color-scheme: dark)");
const paint = () =>
  appWindow.setTitleBarOverlay({
    color: dark.matches ? "#202020" : "#f3f3f3",
    symbolColor: dark.matches ? "#ffffff" : "#1b1b1b",
  });
dark.addEventListener("change", paint);
paint();`,
      },
    ],
    edgeCases: ['Under `titleBarStyle: "default"`, installing the buttons is refused with an advisory.'],
    limitations: OVERLAY_LIMITS,
    related: ["config-window-title-bar-overlay", "chrome", "event-geometrychange"],
  },
  {
    slug: "start-dragging",
    name: "appWindow.startDragging()",
    kind: "method",
    summary: "Starts moving the main window with the mouse, from a `mousedown` handler.",
    platforms: all(OK),
    platformSince: MAC_LINUX_0_3,
    syntax: "appWindow.startDragging(): void",
    returns: "`undefined`.",
    notes: [
      "It hands the press to the platform's window move, which ends when the user releases the button.",
      "On Windows the move starts after the handler returns, and the page keeps running while the window moves: timers, `requestAnimationFrame`, animations and `<sk-surface>` content. It needs the left button still held then; after a release it does nothing.",
      "An `app-region: drag` area does the same without code, and also maximizes on double-click.",
      "Like Tauri's `startDragging()`.",
    ],
    examples: [
      {
        title: "A grip that moves a borderless mini player",
        lang: "vue",
        code: `<script setup lang="ts">
${IMPORT}

function grab(e: MouseEvent) {
  if (e.button === 0) appWindow.startDragging();
}
</script>

<template>
  <div class="grip" @mousedown="grab" />
</template>`,
      },
    ],
    edgeCases: ["Whether the matching `mouseup` reaches the app is not verified."],
    related: ["app-region"],
  },
  {
    slug: "window-controls-overlay",
    name: "navigator.windowControlsOverlay",
    kind: "property",
    summary:
      "The web's Window Controls Overlay API for the main window: where the app's title bar area is, and when it changes.",
    platforms: all(OK),
    mdn: `${MDN}API/Navigator/windowControlsOverlay`,
    syntax:
      "readonly navigator.windowControlsOverlay: WindowControlsOverlay // visible, getTitlebarAreaRect(), ongeometrychange",
    notes: [
      "It describes the main window. A pop-out has no overlay object; use `env(titlebar-area-*)` in its CSS.",
      'Code written for an installed web app with `display_override: ["window-controls-overlay"]` runs unchanged.',
      MAC_TITLEBAR_X,
    ],
    examples: [
      {
        title: "Lay out a search field in the free part of the title bar",
        lang: "ts",
        code: `const overlay = navigator.windowControlsOverlay;

export function fitSearch(search: HTMLElement) {
  const fit = () => {
    const area = overlay.getTitlebarAreaRect();
    search.style.maxWidth = Math.max(0, area.width - 240) + "px";
  };
  overlay.addEventListener("geometrychange", fit);
  fit();
}`,
      },
    ],
    related: ["visible", "get-titlebar-area-rect", "event-geometrychange", "env-titlebar-area"],
    aliases: ["WCO", "WindowControlsOverlay"],
  },
  {
    slug: "visible",
    name: "navigator.windowControlsOverlay.visible",
    kind: "property",
    summary: "Reports whether the app currently owns the title bar strip.",
    platforms: all(OK),
    mdn: `${MDN}API/WindowControlsOverlay/visible`,
    syntax: "readonly navigator.windowControlsOverlay.visible: boolean",
    notes: [
      'It is `true` when `titleBarStyle` is `"hidden"`, the window is neither fullscreen nor minimized, and the host\'s temporary strip is gone.',
      "It does not depend on the `titleBarOverlay` buttons.",
      "It is not reactive; read `appWindow.chrome.overlayVisible` in templates.",
    ],
    examples: [
      {
        title: "Show the app's title bar only while it owns the strip",
        lang: "ts",
        code: `const overlay = navigator.windowControlsOverlay;

export function syncTitlebar(bar: HTMLElement) {
  const sync = () => (bar.hidden = !overlay.visible);
  overlay.addEventListener("geometrychange", sync);
  sync();
}`,
      },
    ],
    related: ["window-controls-overlay", "chrome", "display-mode"],
  },
  {
    slug: "get-titlebar-area-rect",
    name: "navigator.windowControlsOverlay.getTitlebarAreaRect()",
    kind: "method",
    summary: "Returns the part of the title bar strip left to the app's content, in CSS px.",
    platforms: all(OK),
    mdn: `${MDN}API/WindowControlsOverlay/getTitlebarAreaRect`,
    syntax: "navigator.windowControlsOverlay.getTitlebarAreaRect(): DOMRect",
    returns:
      "A plain object with `x`, `y`, `width`, `height`, `top`, `left`, `right`, `bottom` and `toJSON()`. All values are `0` while the overlay is not visible.",
    notes: [
      "`y` is `0`, and `x` is `0` on Windows and Linux. `width` is `innerWidth`, minus 138 px when the `titleBarOverlay` buttons are installed. `height` is the title bar height.",
      MAC_TITLEBAR_X,
      "It reads the state of the last frame.",
    ],
    examples: [
      {
        title: "Size a canvas-drawn title bar",
        lang: "ts",
        code: `export function drawTitlebar(canvas: HTMLCanvasElement, title: string) {
  const area = navigator.windowControlsOverlay.getTitlebarAreaRect();
  canvas.width = Math.round(area.width * devicePixelRatio);
  canvas.height = Math.round(area.height * devicePixelRatio);
  const ctx = canvas.getContext("2d")!;
  ctx.scale(devicePixelRatio, devicePixelRatio);
  ctx.fillText(title, 12, area.height / 2 + 4);
}`,
      },
    ],
    edgeCases: ["It is not a `DOMRect` instance: `instanceof DOMRect` is `false`."],
    related: ["env-titlebar-area", "window-controls-overlay", "event-geometrychange"],
  },
  {
    slug: "ongeometrychange",
    name: "navigator.windowControlsOverlay.ongeometrychange",
    kind: "property",
    summary: "A handler property called when the title bar area changes.",
    platforms: all(OK),
    mdn: `${MDN}API/WindowControlsOverlay/geometrychange_event`,
    syntax: "navigator.windowControlsOverlay.ongeometrychange: ((event: Event) => unknown) | null",
    notes: ["It runs before the listeners added with `addEventListener`."],
    examples: [
      {
        title: "Log title bar changes while debugging",
        lang: "ts",
        code: `navigator.windowControlsOverlay.ongeometrychange = () => {
  console.debug("titlebar", navigator.windowControlsOverlay.getTitlebarAreaRect());
};`,
      },
    ],
    related: ["event-geometrychange"],
  },
  {
    slug: "event-geometrychange",
    name: "geometrychange",
    kind: "event",
    summary: "Fires on `navigator.windowControlsOverlay` when the title bar area may have moved or resized.",
    platforms: all(OK),
    mdn: `${MDN}API/WindowControlsOverlay/geometrychange_event`,
    syntax: 'navigator.windowControlsOverlay.addEventListener("geometrychange", (event: Event) => void)',
    notes: [
      "It fires when the window width, the display scale, maximized or fullscreen, the title bar height, the overlay buttons or the removal of the temporary strip change.",
      "It is a plain `Event`, without the web's `titlebarAreaRect` and `visible` fields; call `getTitlebarAreaRect()`.",
    ],
    examples: [
      {
        title: "Keep a custom title bar in step with the overlay",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const overlay = navigator.windowControlsOverlay;
const width = ref(overlay.getTitlebarAreaRect().width);
const update = () => (width.value = overlay.getTitlebarAreaRect().width);

onMounted(() => overlay.addEventListener("geometrychange", update));
onUnmounted(() => overlay.removeEventListener("geometrychange", update));
</script>

<template>
  <header class="titlebar" :style="{ width: width + 'px' }"><slot /></header>
</template>`,
      },
    ],
    edgeCases: ["A change of the height alone does not fire it.", "Minimizing hides the area without firing it."],
    related: ["ongeometrychange", "get-titlebar-area-rect", "window-controls-overlay", "event-resize"],
  },
  {
    slug: "display-mode",
    name: "@media (display-mode)",
    kind: "property",
    summary: "Matches how the window is shown: with the app's own title bar, with the OS title bar, or fullscreen.",
    platforms: all(OK),
    mdn: `${MDN}CSS/@media/display-mode`,
    syntax: "@media (display-mode: window-controls-overlay | standalone | fullscreen) { … }",
    values: [
      v("window-controls-overlay", OK, 'The window or pop-out has `titleBarStyle` or `chrome` `"hidden"`.'),
      v("standalone", OK, "The OS draws the title bar."),
      v("fullscreen", OK, "The main window is fullscreen."),
      v("browser", OK, "Never matches."),
      v("minimal-ui", OK, "Never matches."),
      v("picture-in-picture", OK, "Never matches."),
    ],
    notes: [
      "Each window answers for itself: CSS in a pop-out is evaluated against the pop-out.",
      "`appWindow.chrome.displayMode` reads the main window's value from JS.",
      "See [display-mode](/css/media-display-mode/) in the CSS reference.",
    ],
    examples: [
      {
        title: "Hide the custom title bar in fullscreen",
        lang: "css",
        code: `.titlebar {
  height: env(titlebar-area-height, 36px);
  app-region: drag;
}

@media (display-mode: fullscreen) {
  .titlebar {
    display: none;
  }
}`,
      },
    ],
    edgeCases: ["A fullscreen pop-out keeps matching its `chrome` mode, not `fullscreen`."],
    related: ["chrome", "config-window-title-bar-style", "sk-window-chrome", "env-titlebar-area"],
  },
  {
    slug: "env-titlebar-area",
    name: "env(titlebar-area-*)",
    kind: "function",
    summary: "CSS variables that give the position and size of the title bar area the app draws in.",
    platforms: all(OK),
    mdn: `${MDN}CSS/env`,
    syntax: "env(titlebar-area-x | titlebar-area-y | titlebar-area-width | titlebar-area-height, <fallback>)",
    notes: [
      "In the main window, they match `getTitlebarAreaRect()` and are defined only while the overlay is visible. Otherwise the fallback applies, or the declaration is dropped without one.",
      'In a pop-out with `chrome="hidden"`, the area is always the pop-out\'s full width by its `titleBarHeight`, also in fullscreen.',
      "`safe-area-inset-*` are always `0px`.",
      "See [env()](/css/function-env/) in the CSS reference.",
    ],
    examples: [
      {
        title: "Fit a custom title bar to the free area",
        lang: "css",
        code: `.titlebar {
  display: flex;
  align-items: center;
  height: env(titlebar-area-height, 36px);
  width: env(titlebar-area-width, 100%);
  app-region: drag;
}`,
        note: "The fallbacks apply in fullscreen and with the OS title bar.",
      },
    ],
    edgeCases: [
      "`titlebar-area-y` is always `0px`. In the main window, `titlebar-area-x` is `0px` on Windows and Linux, and starts past the traffic lights on macOS. The default 78 px inset follows a custom button position.",
    ],
    related: ["get-titlebar-area-rect", "config-window-title-bar-height", "sk-window-title-bar-height", "display-mode"],
    aliases: ["titlebar-area-height", "titlebar-area-width"],
  },
  {
    slug: "app-region",
    name: "app-region",
    kind: "property",
    summary: "Marks the elements by which the user drags the window, as in an installed web app.",
    platforms: all(OK),
    platformSince: { macos: "0.3.0" },
    web: true,
    syntax: "app-region: drag | no-drag;\n-webkit-app-region: drag | no-drag;\nwindow-drag: move | none;",
    values: [
      v("drag", OK, "The element and its descendants drag the window. `move` is the `window-drag` spelling."),
      v("no-drag", OK, "Takes the element out of an ancestor's drag region. `none` is the `window-drag` spelling."),
    ],
    notes: [
      "It inherits: every descendant drags until one sets `no-drag`.",
      "On Windows, the OS moves the window, with double-click to maximize and Aero Snap.",
      "On macOS, the host moves the window, and a double-click does what the user chose in System Settings for a title bar double-click.",
      "On Linux, the host moves the window, and a double-click toggles maximize.",
      "Clicks inside a drag region still reach the element, unlike in a browser; `sk dev` advises `no-drag` on elements with click listeners there.",
      'It works in a pop-out with `chrome="hidden"`.',
      "The first drag region the app paints removes the host's temporary title bar strip.",
      "See [app-region](/css/app-region/) in the CSS reference.",
    ],
    examples: [
      {
        title: "A draggable title bar with a search field",
        lang: "vue",
        code: `<template>
  <header class="titlebar">
    <span class="app-name">Atlas</span>
    <input class="search" placeholder="Search" />
  </header>
</template>

<style scoped>
.titlebar {
  display: flex;
  align-items: center;
  height: env(titlebar-area-height, 36px);
  app-region: drag;
}
.search {
  app-region: no-drag;
}
</style>`,
      },
    ],
    edgeCases: ['Under `titleBarStyle: "default"`, it does nothing, with an advisory.'],
    related: ["window-control", "start-dragging", "config-window-title-bar-fallback", "env-titlebar-area"],
    aliases: ["-webkit-app-region", "window-drag", "drag region"],
  },
  {
    slug: "window-control",
    name: "--window-control",
    kind: "property",
    summary: "Turns an element into the window's minimize, maximize or close button.",
    platforms: all(OK),
    platformSince: { macos: "0.3.0" },
    syntax: "--window-control: minimize | maximize | close | none;",
    values: [
      v("minimize", OK, "Minimizes the window."),
      v("maximize", OK, "Maximizes or restores the window. On Windows 11, hovering it opens the snap layouts flyout."),
      v("close", OK, "Closes the window as the user's close does."),
      v("none", OK, "Removes an earlier role."),
    ],
    notes: [
      "The host performs the action, so no click handler is needed. On Windows the OS performs it; on macOS and Linux the host does when the mouse button is released.",
      "In the main window, a `close` goes through `close-requested`, `beforeunload` and `pagehide`, as the OS close button does. In a pop-out, it closes the pop-out.",
      "A click dispatched from code, such as `el.click()` on the element or on a descendant, performs the action too.",
      "It follows `resizable`, `minimizable` and `maximizable`: with `false`, the matching button no longer acts.",
      "On macOS, the traffic lights are already the window's buttons; most apps draw none of their own there.",
      "See [--window-control](/css/window-control/) in the CSS reference.",
    ],
    examples: [
      {
        title: "Custom caption buttons",
        lang: "vue",
        code: `<template>
  <div class="controls">
    <button class="min" aria-label="Minimize">–</button>
    <button class="max" aria-label="Maximize">□</button>
    <button class="close" aria-label="Close">✕</button>
  </div>
</template>

<style scoped>
.controls button {
  width: 46px;
  height: env(titlebar-area-height, 32px);
}
.min { --window-control: minimize; }
.max { --window-control: maximize; }
.close { --window-control: close; }
.close:hover { background-color: #c42b1c; }
</style>`,
      },
    ],
    edgeCases: [
      "Do not add a click handler that does the same: the window would act twice. `sk dev` warns about it.",
      'Under `titleBarStyle: "default"`, it does nothing, with an advisory.',
    ],
    related: ["app-region", "config-window-title-bar-overlay", "event-close-requested", "is-maximized"],
    aliases: ["window buttons", "caption buttons"],
  },
]);

const SK_IMPORT = 'import { SkWindow } from "@skathi-native/runtime";';
const LIVE = "Live: a change on a mounted `<SkWindow>` applies to the open window, shortly after the change.";
const CREATION =
  "Creation-time: only the value at mount counts. To change it, close and reopen the window, for example with a `:key`, or `{#key}` in Svelte.";
const POPOUT_HWND_ONLY = "On macOS and Linux, it does nothing.";

const popouts = section("Pop-out windows", [
  {
    slug: "sk-window-max-width",
    name: "<SkWindow maxWidth>",
    kind: "prop",
    summary: "Limits the pop-out’s client width in CSS pixels.",
    platforms: all(OK),
    since: "0.6.0",
    syntax: '<SkWindow :max-width="640" />',
    notes: ["Live. Omitting the prop restores the window’s default."],
  },
  {
    slug: "sk-window-max-height",
    name: "<SkWindow maxHeight>",
    kind: "prop",
    summary: "Limits the pop-out’s client height in CSS pixels.",
    platforms: all(OK),
    since: "0.6.0",
    syntax: '<SkWindow :max-height="360" />',
    notes: ["Live. Omitting the prop restores the window’s default."],
  },
  {
    slug: "sk-window-aspect-ratio",
    name: "<SkWindow aspectRatio>",
    kind: "prop",
    summary: "Locks the pop-out’s client aspect ratio during resize.",
    platforms: all(OK),
    since: "0.6.0",
    syntax: '<SkWindow :aspect-ratio="16 / 9" />',
    notes: ["Live. Omitting the prop restores the window’s default."],
  },
  {
    slug: "sk-window-opacity",
    name: "<SkWindow opacity>",
    kind: "prop",
    summary: "Applies native window alpha between 0 and 1.",
    platforms: os(OK, OK, PART),
    since: "0.6.0",
    syntax: '<SkWindow :opacity="0.8" />',
    notes: ["Live. Omitting the prop restores the window’s default."],
  },
  {
    slug: "sk-window-visible-on-all-workspaces",
    name: "<SkWindow visibleOnAllWorkspaces>",
    kind: "prop",
    summary: "Lets a floating mini player join fullscreen Spaces on macOS. X11 requests all desktops.",
    platforms: os(NO, OK, PART),
    since: "0.6.0",
    syntax: "<SkWindow always-on-top visible-on-all-workspaces />",
    notes: ["Live. Omitting the prop restores the window’s default."],
  },
  {
    slug: "sk-window-traffic-light-position",
    name: "<SkWindow trafficLightPosition>",
    kind: "prop",
    summary: "Overrides the close button’s title-bar position on macOS.",
    platforms: os(NO, OK, NO),
    since: "0.6.0",
    syntax: '<SkWindow :traffic-light-position="{ x: 14, y: 8 }" />',
    notes: ["Live. Omitting the prop restores the window’s default."],
  },
  {
    slug: "sk-window",
    name: "<SkWindow>",
    kind: "component",
    summary: "Opens a second OS window that paints part of the same component tree.",
    platforms: os(OK, PART, PART),
    syntax: `${SK_IMPORT}\n\n<SkWindow v-if="open" title="Palette" :bounds="{ width: 320, height: 400 }" @close="open = false">\n  …\n</SkWindow>`,
    notes: [
      "Mounting it opens the window on the next tick, and unmounting it closes the window. Drive it with `v-if`, or `{#if}` in Svelte.",
      "The slot stays in the same component tree, with the same reactive state: a click in the pop-out runs the same handler as in the main window. Nothing is copied or messaged.",
      'The content stops painting in the main window at once. It is wrapped in a `<div class="sk-window-root">` that fills the pop-out and receives the component\'s attributes.',
      "CSS inside it, such as `@media`, `position: fixed` and `env(titlebar-area-*)`, answers for the pop-out. `window.innerWidth`, `matchMedia()` and `appWindow` still answer for the main window.",
      "Focus is per window: an `<input>` in the pop-out keeps its focus while the main window keeps its own.",
      "All pop-outs close when the main window closes. A pop-out cannot cancel its own close.",
      "It follows `appWindow.theme`.",
      "In Svelte, import it from `@skathi-native/runtime/svelte`. The props keep their camelCase names, the events are the callback props `onopen`, `onclose`, `onfocus`, `onblur`, `onresize`, `onshow` and `onhide`, and the exposed values are read with `bind:el`, `bind:windowId`, `bind:width`, `bind:height` and `bind:scale`. `bind:this` gives `mapFromMain()`.",
    ],
    examples: [
      {
        title: "A floating color palette",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const showPalette = ref(false);
const brush = ref("charcoal");
const colors = ["charcoal", "rust", "moss", "slate"];
</script>

<template>
  <button @click="showPalette = !showPalette">Palette</button>
  <canvas class="sheet" :data-brush="brush" />

  <SkWindow v-if="showPalette" title="Palette" :bounds="{ width: 240, height: 320 }" always-on-top @close="showPalette = false">
    <button v-for="c in colors" :key="c" :class="{ on: brush === c }" @click="brush = c">{{ c }}</button>
  </SkWindow>
</template>`,
      },
    ],
    edgeCases: [
      "If the app does not unmount the component in `@close`, the content paints in the main window again.",
      "A `<SkWindow>` inside another is opened as a sibling window, with an advisory.",
      "The same element cannot be painted by two windows; a second open is refused.",
      "A `<Teleport :to>` into its `el` works, which moves existing content into the pop-out. Svelte has no `<Teleport>`: put the content in the children, or mount a component into `el` with Svelte's `mount(Comp, { target: el })`.",
    ],
    limitations: [
      "On macOS, `monitor` does nothing. On Linux, `taskbar` and `monitor` do nothing, `focusable` only keeps the window from being activated when it is shown, and `mapFromMain()` returns `null`. Each prop lists its other gaps, mostly under Wayland.",
    ],
    related: ["sk-window-event-close", "sk-window-bounds", "sk-window-el", "window-open"],
    aliases: ["SkWindow", "second window", "multi-window", "BrowserWindow"],
  },
  {
    slug: "sk-window-title",
    name: "<SkWindow title>",
    kind: "prop",
    summary: "Sets the pop-out's OS title.",
    platforms: all(OK),
    syntax: '<SkWindow title="Palette" />',
    params: [{ name: "title", type: "string", description: "The OS caption.", default: '`"Skathi"`', optional: true }],
    notes: [LIVE],
    examples: [
      {
        title: "Show the current tool in the palette's title",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, ref } from "vue";
${SK_IMPORT}

const tool = ref("Brush");
const title = computed(() => "Palette - " + tool.value);
</script>

<template>
  <SkWindow :title="title">
    <button @click="tool = 'Brush'">Brush</button>
    <button @click="tool = 'Eraser'">Eraser</button>
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window", "title"],
  },
  {
    slug: "sk-window-bounds",
    name: "<SkWindow bounds>",
    kind: "prop",
    summary: "Sets where the pop-out opens and how big its client area is, in CSS px.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: '<SkWindow :bounds="{ x: 60, y: 80, width: 320, height: 400 }" />',
    params: [
      { name: "x", type: "number", description: "Left edge on the screen.", optional: true },
      { name: "y", type: "number", description: "Top edge on the screen.", optional: true },
      { name: "width", type: "number", description: "Client width.", default: "`420`", optional: true },
      { name: "height", type: "number", description: "Client height.", default: "`320`", optional: true },
    ],
    notes: [
      "Without both `x` and `y`, the pop-out opens centered on the main window's monitor.",
      "`x` and `y` are CSS px at the main window's scale. On Windows, the pop-out opens on the monitor they fall on, also one other than the primary.",
      "It is live: a change moves or resizes the open window, field by field. The values are not clamped.",
      "A change of another prop keeps the place where the user moved the window.",
      'It is ignored with `cover="monitor"`.',
    ],
    examples: [
      {
        title: "Open an inspector beside the main window",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const open = ref(false);
const bounds = ref({ x: 0, y: 0, width: 300, height: 480 });

function show() {
  bounds.value = { ...bounds.value, x: screenX + outerWidth + 8, y: screenY };
  open.value = true;
}
</script>

<template>
  <button @click="show">Inspector</button>
  <SkWindow v-if="open" title="Inspector" :bounds="bounds" @close="open = false"><slot /></SkWindow>
</template>`,
      },
    ],
    edgeCases: [
      "A `width` or `height` of `0` or less is ignored in a live change.",
      "There is no getter for the current position. The size comes from `@resize`.",
    ],
    limitations: ["On Linux under Wayland, `x` and `y` are ignored: the compositor places the window."],
    related: ["sk-window-event-resize", "sk-window-min-width", "screen-x"],
  },
  {
    slug: "sk-window-chrome",
    name: "<SkWindow chrome>",
    kind: "prop",
    summary: "Chooses between the OS title bar and a title bar the app draws in the pop-out.",
    platforms: all(OK),
    syntax: '<SkWindow chrome="default" | "hidden" />',
    values: [
      v("default", OK, "The OS draws the title bar. `display-mode` is `standalone`."),
      v("hidden", OK, "The app draws the title bar. `display-mode` is `window-controls-overlay`."),
    ],
    notes: [
      CREATION,
      'With `"hidden"`, the pop-out has no host buttons and no temporary strip: the app paints an `app-region: drag` area and its own `--window-control` buttons.',
      'On macOS, with `"hidden"`, the traffic lights stay, centered in `titleBarHeight`.',
      "A change prints an advisory.",
    ],
    examples: [
      {
        title: "A sticky note with its own drag strip",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const open = ref(true);
</script>

<template>
  <SkWindow v-if="open" chrome="hidden" :title-bar-height="28" :bounds="{ width: 260, height: 220 }" @close="open = false">
    <header class="strip">
      <button class="close" aria-label="Close">✕</button>
    </header>
    <textarea class="note" />
  </SkWindow>
</template>

<style scoped>
.strip { height: env(titlebar-area-height, 28px); app-region: drag; display: flex; justify-content: flex-end; }
.close { --window-control: close; width: 36px; }
</style>`,
      },
    ],
    related: ["sk-window-title-bar-height", "app-region", "window-control", "display-mode"],
  },
  {
    slug: "sk-window-backdrop",
    name: "<SkWindow backdrop>",
    kind: "prop",
    summary: "Sets the material behind the pop-out's transparent content.",
    platforms: os(OK, PART, PART),
    syntax: '<SkWindow backdrop="mica" | "mica-alt" | "acrylic" | "transparent" | "none" />',
    params: [
      {
        name: "backdrop",
        type: "string",
        description: "The same names as `appWindow.backdrop`, synonyms included.",
        default: "`window.backdrop` from `sk.config.json`",
        optional: true,
      },
    ],
    notes: [
      LIVE,
      "An unknown name prints an advisory and keeps the default.",
      "The default is the startup backdrop, even after the app changes `appWindow.backdrop`. Removing the prop goes back to it.",
    ],
    examples: [
      {
        title: "A translucent quick-search window",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}
</script>

<template>
  <SkWindow title="Search" backdrop="acrylic" :bounds="{ width: 560, height: 64 }" :activate="true">
    <input class="query" placeholder="Search files" />
  </SkWindow>
</template>`,
      },
    ],
    limitations: [
      "On macOS, the three materials draw the same blur.",
      "On Linux, the three materials draw an opaque fill.",
    ],
    related: ["backdrop", "config-window-backdrop"],
  },
  {
    slug: "sk-window-always-on-top",
    name: "<SkWindow alwaysOnTop>",
    kind: "prop",
    summary: "Keeps the pop-out above other windows.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: '<SkWindow :always-on-top="true" />',
    params: [
      {
        name: "alwaysOnTop",
        type: "boolean",
        description: "Whether the pop-out stays on top.",
        default: "`false`",
        optional: true,
      },
    ],
    notes: [LIVE, "The pop-out is not activated by it. It is applied again each time the window is shown."],
    examples: [
      {
        title: "A pinnable mini player",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const pinned = ref(true);
</script>

<template>
  <SkWindow title="Now playing" :always-on-top="pinned" :bounds="{ width: 320, height: 180 }">
    <video src="assets/episode.mp4" autoplay />
    <button @click="pinned = !pinned">{{ pinned ? "Unpin" : "Pin" }}</button>
  </SkWindow>
</template>`,
      },
    ],
    limitations: ["On Linux under Wayland, it does nothing."],
    related: ["always-on-top", "sk-window-taskbar"],
  },
  {
    slug: "sk-window-resizable",
    name: "<SkWindow resizable>",
    kind: "prop",
    summary: "Sets whether the user can resize the pop-out.",
    platforms: all(OK),
    syntax: '<SkWindow :resizable="false" />',
    params: [
      {
        name: "resizable",
        type: "boolean",
        description: "`false` removes the resize edges.",
        default: "`true`",
        optional: true,
      },
    ],
    notes: [CREATION, "A change prints an advisory."],
    examples: [
      {
        title: "A fixed-size color picker",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}
</script>

<template>
  <SkWindow title="Pick a color" :resizable="false" :bounds="{ width: 280, height: 320 }">
    <input type="color" />
  </SkWindow>
</template>`,
      },
    ],
    related: ["config-window-resizable", "sk-window-min-width"],
  },
  {
    slug: "sk-window-min-width",
    name: "<SkWindow minWidth>",
    kind: "prop",
    summary: "Sets the narrowest the user can make the pop-out, in CSS px.",
    platforms: all(OK),
    syntax: '<SkWindow :min-width="240" />',
    params: [{ name: "minWidth", type: "number", description: "Minimum client width.", optional: true }],
    notes: ["Applies at creation and live. The missing side is unconstrained."],
    examples: [
      {
        title: "Keep the palette's swatches readable",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}
</script>

<template>
  <SkWindow title="Palette" :min-width="240" :min-height="200">
    <slot />
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-min-height", "config-window-min-width"],
  },
  {
    slug: "sk-window-min-height",
    name: "<SkWindow minHeight>",
    kind: "prop",
    summary: "Sets the shortest the user can make the pop-out, in CSS px.",
    platforms: all(OK),
    syntax: '<SkWindow :min-height="200" />',
    params: [{ name: "minHeight", type: "number", description: "Minimum client height.", optional: true }],
    notes: ["Applies at creation and live. The missing side is unconstrained."],
    examples: [
      {
        title: "Leave room for a player's controls",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}
</script>

<template>
  <SkWindow title="Player" :min-height="160">
    <video src="assets/clip.mp4" controls />
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-min-width", "config-window-min-height"],
  },
  {
    slug: "sk-window-title-bar-height",
    name: "<SkWindow titleBarHeight>",
    kind: "prop",
    summary: "Sets the height of the title bar area in a pop-out that draws its own title bar.",
    platforms: all(OK),
    syntax: '<SkWindow chrome="hidden" :title-bar-height="28" />',
    params: [
      { name: "titleBarHeight", type: "number", description: "Height in CSS px.", default: "`32`", optional: true },
    ],
    notes: [
      LIVE,
      'It is `env(titlebar-area-height)` inside the pop-out when `chrome` is `"hidden"`.',
      "On macOS the lights follow height changes while automatic positioning is active.",
    ],
    examples: [
      {
        title: "A compact title bar in a tool window",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}
</script>

<template>
  <SkWindow chrome="hidden" :title-bar-height="24" title="Layers">
    <header class="bar">Layers</header>
    <slot />
  </SkWindow>
</template>

<style scoped>
.bar { height: env(titlebar-area-height, 24px); app-region: drag; font-size: 11px; }
</style>`,
      },
    ],
    related: ["sk-window-chrome", "env-titlebar-area"],
  },
  {
    slug: "sk-window-activate",
    name: "<SkWindow activate>",
    kind: "prop",
    summary: "Sets whether the pop-out takes the keyboard focus when it is shown.",
    platforms: all(OK),
    syntax: '<SkWindow :activate="true" />',
    params: [
      {
        name: "activate",
        type: "boolean",
        description: "`true` activates the pop-out on show.",
        default: "`false`",
        optional: true,
      },
    ],
    notes: [
      "Live: it is read at each show, the first one and each later one through `visible`.",
      "With the default `false`, the pop-out appears without taking focus, so a palette does not steal the caret from the main window.",
      "It is ignored, with an advisory, when `focusable` is `false`.",
    ],
    examples: [
      {
        title: "A command window that takes the keyboard",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const open = ref(false);
</script>

<template>
  <button @click="open = true">Run command…</button>
  <SkWindow v-if="open" title="Run" activate :bounds="{ width: 480, height: 60 }" @close="open = false">
    <input class="command" autofocus @keydown.esc="open = false" />
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-focusable", "sk-window-visible", "sk-window-event-focus"],
  },
  {
    slug: "sk-window-visible",
    name: "<SkWindow visible>",
    kind: "prop",
    summary: "Hides or shows the pop-out while keeping it open and painted.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: '<SkWindow :visible="shown" />',
    params: [
      { name: "visible", type: "boolean", description: "`false` hides the window.", default: "`true`", optional: true },
    ],
    notes: [
      LIVE,
      "A hidden pop-out keeps painting, so showing it again takes one frame instead of opening a window.",
      "On show, the window is placed, painted, then shown, and `@show` fires with the time to the first frame.",
      "On hide, activation returns to the main window and `@hide` fires.",
    ],
    examples: [
      {
        title: "A quick-look preview that appears at once",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

defineProps<{ file: string }>();
const peeking = ref(false);
</script>

<template>
  <li @keydown.space.prevent="peeking = !peeking" tabindex="0">{{ file }}</li>
  <SkWindow title="Quick Look" :visible="peeking" :bounds="{ width: 640, height: 480 }">
    <img :src="file" class="preview" />
  </SkWindow>
</template>`,
      },
    ],
    limitations: [
      "On Linux under Wayland, a window cannot be hidden: `false` minimizes it, and a pop-out that starts hidden appears when it opens.",
    ],
    related: ["sk-window-event-show", "sk-window-event-hide", "sk-window-activate"],
  },
  {
    slug: "sk-window-cover",
    name: "<SkWindow cover>",
    kind: "prop",
    summary: "Makes the pop-out a borderless window that covers a whole monitor, taskbar included.",
    platforms: os(OK, PART, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: '<SkWindow cover="monitor" />',
    params: [
      {
        name: "cover",
        type: '"monitor"',
        description: "Covers the main window's monitor, or the one `monitor` names.",
        optional: true,
      },
    ],
    notes: [
      CREATION,
      "`bounds` is ignored. The window cannot be minimized.",
      "It waits hidden and is sized to the monitor each time it is shown.",
      "It counts as fullscreen for `requestFullscreen()` inside it.",
      "An unknown value prints an advisory and uses `bounds`. A change prints an advisory.",
    ],
    examples: [
      {
        title: "A click-through overlay on the whole monitor",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}

defineProps<{ countdown: number }>();
</script>

<template>
  <SkWindow cover="monitor" backdrop="transparent" always-on-top ignore-mouse-events :focusable="false">
    <div class="countdown">{{ countdown }}</div>
  </SkWindow>
</template>

<style scoped>
.countdown { position: fixed; right: 48px; bottom: 48px; font-size: 64px; }
</style>`,
      },
    ],
    limitations: [
      "On macOS and Linux, each show sizes the window to the whole display it is on, but the window is not made borderless, and `monitor` is ignored.",
      "On Linux under Wayland, the window cannot be placed or sized to the display.",
    ],
    related: ["sk-window-monitor", "sk-window-ignore-mouse-events", "sk-window-focusable"],
    aliases: ["overlay", "kiosk"],
  },
  {
    slug: "sk-window-taskbar",
    name: "<SkWindow taskbar>",
    kind: "prop",
    summary: "Sets whether the pop-out has its own taskbar button or belongs to the main window.",
    platforms: os(OK, OK, NO),
    syntax: '<SkWindow :taskbar="false" />',
    params: [
      {
        name: "taskbar",
        type: "boolean",
        description: "`false` ties the pop-out to the main window.",
        default: "`true`",
        optional: true,
      },
    ],
    notes: [
      CREATION,
      "On Windows, with `false`, the main window owns the pop-out: it has no taskbar button and no Alt+Tab entry, stays above the main window and minimizes with it.",
      "On macOS, with `false`, the pop-out is a child window of the main window while it is shown: it stays above the main window, moves when the main window is dragged, and hides and minimizes with it. Unlike an owned window on Windows, it also moves with the main window. While the pop-out is hidden it is not attached, so showing the main window again does not show it.",
      "A change prints an advisory.",
    ],
    examples: [
      {
        title: "A find bar that belongs to the main window",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}
</script>

<template>
  <SkWindow title="Find" :taskbar="false" :bounds="{ width: 360, height: 56 }" activate>
    <input class="find" placeholder="Find in document" />
  </SkWindow>
</template>`,
      },
    ],
    limitations: ["On Linux, it does nothing."],
    related: ["sk-window-focusable", "hide"],
    aliases: ["skipTaskbar", "owned window", "tool window"],
  },
  {
    slug: "sk-window-low-latency-surface",
    name: "<SkWindow lowLatencySurface>",
    kind: "prop",
    summary: "Enables on-demand Metal presentation for this macOS pop-out.",
    since: "0.6.0",
    platforms: os(NO, OK, NO),
    syntax: '<SkWindow :low-latency-surface="true" />',
    notes: ["Defaults to false. Set the prop when opening the window. Other windows keep their own frame pacing."],
    related: ["config-window-low-latency-surface"],
  },
  {
    slug: "sk-window-full-resolution",
    name: "<SkWindow fullResolution>",
    kind: "prop",
    summary: "Keeps images in the pop-out at their full decoded resolution.",
    platforms: all(OK),
    syntax: '<SkWindow :full-resolution="true" />',
    params: [
      {
        name: "fullResolution",
        type: "boolean",
        description: "`true` keeps the full decode.",
        default: "`false`",
        optional: true,
      },
    ],
    notes: [
      LIVE,
      "By default, an image drawn small keeps a reduced copy in memory. With `true`, images shown in this window keep the full decode.",
      "It takes effect at the next memory sweep.",
    ],
    examples: [
      {
        title: "A photo viewer that zooms without blur",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}

defineProps<{ photo: string }>();
</script>

<template>
  <SkWindow title="Viewer" full-resolution :bounds="{ width: 1024, height: 768 }">
    <img :src="photo" class="photo" />
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window"],
  },
  {
    slug: "sk-window-ignore-mouse-events",
    name: "<SkWindow ignoreMouseEvents>",
    kind: "prop",
    summary: "Lets every mouse event pass through the pop-out to the windows under it.",
    platforms: os(OK, OK, PART),
    platformSince: MAC_LINUX_0_3,
    syntax: '<SkWindow :ignore-mouse-events="true" />',
    params: [
      {
        name: "ignoreMouseEvents",
        type: "boolean",
        description: "`true` makes the window click-through.",
        default: "`false`",
        optional: true,
      },
    ],
    notes: [
      LIVE,
      "Clicks reach the windows of other apps under the pop-out.",
      "Like Electron's `setIgnoreMouseEvents(true)`.",
    ],
    examples: [
      {
        title: "Toggle click-through on a desktop widget",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const locked = ref(true);
</script>

<template>
  <label><input type="checkbox" v-model="locked" /> Lock the widget</label>
  <SkWindow title="CPU" backdrop="transparent" always-on-top :ignore-mouse-events="locked" :bounds="{ width: 200, height: 120 }">
    <slot />
  </SkWindow>
</template>`,
        note: "Keep the toggle in the main window: a click-through window cannot receive the click that unlocks it.",
      },
    ],
    limitations: ["On Linux under X11, it does nothing. Under Wayland it works."],
    related: ["sk-window-cover", "sk-window-focusable"],
    aliases: ["click-through", "setIgnoreMouseEvents"],
  },
  {
    slug: "sk-window-focusable",
    name: "<SkWindow focusable>",
    kind: "prop",
    summary: "Sets whether the pop-out can ever become the active window.",
    platforms: os(OK, OK, PART),
    syntax: '<SkWindow :focusable="false" />',
    params: [
      {
        name: "focusable",
        type: "boolean",
        description: "`false` never activates the window.",
        default: "`true`",
        optional: true,
      },
    ],
    notes: [
      CREATION,
      "With `false`, a click reaches the window without activating it: the main window stays active and gets no `blur`, and keys keep going to the main window. Route arrows and Enter to the pop-out's content yourself, as a menu does.",
      "On Windows, the window also has no taskbar or Alt+Tab entry.",
      "On macOS, the window is a nonactivating panel at the normal window level; use `alwaysOnTop` to keep it above other apps' windows. Hover and the cursor work in it while it is not the key window, and it stays visible when the app is not active.",
      "A change prints an advisory.",
      "Like Electron's `focusable: false`.",
    ],
    examples: [
      {
        title: "A heads-up display that never takes focus",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}

defineProps<{ fps: number }>();
</script>

<template>
  <SkWindow title="HUD" :focusable="false" always-on-top :bounds="{ x: 24, y: 24, width: 160, height: 48 }">
    <span class="fps">{{ fps }} fps</span>
  </SkWindow>
</template>`,
      },
    ],
    limitations: [
      "On Linux, it only keeps the pop-out from being activated when it is shown; a click still activates it.",
    ],
    related: ["sk-window-activate", "sk-window-taskbar", "sk-window-ignore-mouse-events"],
  },
  {
    slug: "sk-window-content-protection",
    name: "<SkWindow contentProtection>",
    kind: "prop",
    summary: "Keeps the pop-out out of screenshots and screen sharing.",
    platforms: os(OK, OK, NO),
    platformSince: { macos: "0.3.0" },
    syntax: '<SkWindow :content-protection="true" />',
    params: [
      {
        name: "contentProtection",
        type: "boolean",
        description: "`true` excludes the window from capture.",
        default: "`false`",
        optional: true,
      },
    ],
    notes: [
      LIVE,
      "On Windows 10 version 2004 and later, the window is left out of captures. On earlier versions it is captured as black.",
      "On macOS, it sets the window's sharing type to none.",
      "Like Electron's `setContentProtection(true)`.",
    ],
    examples: [
      {
        title: "Hide a password reveal window from screen sharing",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}

defineProps<{ secret: string }>();
</script>

<template>
  <SkWindow title="Password" content-protection :bounds="{ width: 320, height: 120 }">
    <code class="secret">{{ secret }}</code>
  </SkWindow>
</template>`,
      },
    ],
    limitations: ["On Linux, it does nothing."],
    related: ["sk-window"],
  },
  {
    slug: "sk-window-monitor",
    name: "<SkWindow monitor>",
    kind: "prop",
    summary: 'Chooses which monitor a `cover="monitor"` pop-out covers.',
    platforms: os(OK, NO, NO),
    syntax: '<SkWindow cover="monitor" :monitor="1" />',
    params: [
      {
        name: "monitor",
        type: "number",
        description: "A monitor index, from `0`, in the order Windows lists the monitors.",
        default: "the main window's monitor",
        optional: true,
      },
    ],
    notes: [
      LIVE,
      "A shown cover window moves to the new monitor.",
      "An index with no monitor prints an advisory and uses the main window's monitor.",
      "It is ignored without `cover`.",
    ],
    examples: [
      {
        title: "A presenter view on the second monitor",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}

defineProps<{ slide: string }>();
const extended = screen.isExtended;
</script>

<template>
  <SkWindow v-if="extended" cover="monitor" :monitor="1" title="Presentation">
    <img :src="slide" class="slide" />
  </SkWindow>
</template>`,
      },
    ],
    edgeCases: ["The index is not related to `screen` or to any web API."],
    limitations: [POPOUT_HWND_ONLY],
    related: ["sk-window-cover", "screen"],
  },
  {
    slug: "sk-window-event-open",
    name: "<SkWindow @open>",
    kind: "event",
    summary: "Fires when the pop-out's OS window exists.",
    platforms: all(OK),
    syntax: '<SkWindow @open="(windowId: number) => void" />',
    params: [
      {
        name: "windowId",
        type: "number",
        description: "The window's id, the one the devtools control script's `--window` takes.",
      },
    ],
    notes: ["Live prop changes start reaching the window from here.", "In Svelte, it is the `onopen` callback prop."],
    examples: [
      {
        title: "Log the id to target the window in devtools",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}
</script>

<template>
  <SkWindow title="Inspector" @open="(id) => console.info('inspector window', id)">
    <slot />
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-window-id", "sk-window-event-close"],
  },
  {
    slug: "sk-window-event-close",
    name: "<SkWindow @close>",
    kind: "event",
    summary: "Fires when the pop-out closes: by the user, by the OS, or because the app unmounts the component.",
    platforms: all(OK),
    syntax: '<SkWindow @close="() => void" />',
    notes: [
      "After a close by the user or the OS, unmount the component in the handler, usually by setting its `v-if` flag to `false`. Otherwise the content paints in the main window again.",
      "When the app unmounts the component, it fires synchronously as the unmount starts, before the OS window is gone.",
      "It fires at most once per window, and only if the window had opened.",
      "After it, `windowId` is `0` and prop changes no longer reach a window.",
      "In Svelte, it is the `onclose` callback prop.",
    ],
    examples: [
      {
        title: "Close the pop-out from its X button",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const detached = ref(false);
</script>

<template>
  <button v-if="!detached" @click="detached = true">Detach player</button>
  <SkWindow v-if="detached" title="Player" @close="detached = false">
    <video src="assets/clip.mp4" autoplay />
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window", "sk-window-event-open"],
  },
  {
    slug: "sk-window-event-focus",
    name: "<SkWindow @focus>",
    kind: "event",
    summary: "Fires when the pop-out becomes the active window.",
    platforms: all(OK),
    syntax: '<SkWindow @focus="() => void" />',
    notes: ["It arrives with the pop-out's next frame.", "In Svelte, it is the `onfocus` callback prop."],
    examples: [
      {
        title: "Highlight the palette while it is active",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const active = ref(false);
</script>

<template>
  <SkWindow title="Palette" @focus="active = true" @blur="active = false">
    <div class="palette" :class="{ active }"><slot /></div>
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-event-blur", "event-focus", "sk-window-activate"],
  },
  {
    slug: "sk-window-event-blur",
    name: "<SkWindow @blur>",
    kind: "event",
    summary: "Fires when the pop-out stops being the active window.",
    platforms: all(OK),
    syntax: '<SkWindow @blur="() => void" />',
    notes: ["It arrives with the pop-out's next frame.", "In Svelte, it is the `onblur` callback prop."],
    examples: [
      {
        title: "Close a quick-search window when the user clicks away",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const open = ref(true);
</script>

<template>
  <SkWindow v-if="open" title="Search" activate @blur="open = false" @close="open = false">
    <input class="query" autofocus />
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-event-focus", "event-blur"],
  },
  {
    slug: "sk-window-event-resize",
    name: "<SkWindow @resize>",
    kind: "event",
    summary: "Fires with the pop-out's size and scale when it first paints and whenever they change.",
    platforms: all(OK),
    syntax: '<SkWindow @resize="(metrics: SkWindowMetrics) => void" />',
    params: [
      { name: "metrics.width", type: "number", description: "The pop-out's content width in its CSS px." },
      { name: "metrics.height", type: "number", description: "The pop-out's content height in its CSS px." },
      { name: "metrics.scale", type: "number", description: "The pop-out's own display scale." },
    ],
    notes: [
      "It is the only way to read a pop-out's size from JS: `innerWidth` is the main window's.",
      "It also updates the exposed `width`, `height` and `scale`.",
      "In Svelte, it is the `onresize` callback prop.",
    ],
    examples: [
      {
        title: "Switch the palette to a grid when it is wide",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
import { SkWindow, type SkWindowMetrics } from "@skathi-native/runtime";

const wide = ref(false);
const onResize = (m: SkWindowMetrics) => (wide.value = m.width > 360);
</script>

<template>
  <SkWindow title="Palette" @resize="onResize">
    <div :class="wide ? 'grid' : 'list'"><slot /></div>
  </SkWindow>
</template>`,
        note: "`@media (min-width: 360px)` inside the pop-out does the same from CSS.",
      },
    ],
    related: ["sk-window-width", "sk-window-scale", "event-resize"],
    aliases: ["SkWindowMetrics"],
  },
  {
    slug: "sk-window-event-show",
    name: "<SkWindow @show>",
    kind: "event",
    summary: "Fires when the pop-out appears, with the time it took to paint.",
    platforms: all(OK),
    platformSince: MAC_LINUX_0_3,
    syntax:
      '<SkWindow @show="(shown: { firstFrameMs: number; width: number; height: number; scale: number }) => void" />',
    params: [
      {
        name: "shown.firstFrameMs",
        type: "number",
        description: "Milliseconds from the open or the show request to the first frame.",
      },
      { name: "shown.width", type: "number", description: "Client width in CSS px." },
      { name: "shown.height", type: "number", description: "Client height in CSS px." },
      { name: "shown.scale", type: "number", description: "The pop-out's display scale." },
    ],
    notes: [
      "It fires once when the pop-out first appears, whether it opens visible or starts hidden, and again each time `visible` goes from `false` to `true`.",
      "In Svelte, it is the `onshow` callback prop.",
    ],
    examples: [
      {
        title: "Measure how fast the overlay appears",
        lang: "vue",
        code: `<script setup lang="ts">
${SK_IMPORT}

defineProps<{ shown: boolean }>();
</script>

<template>
  <SkWindow cover="monitor" :visible="shown" @show="(s) => console.info('overlay in', Math.round(s.firstFrameMs), 'ms')">
    <slot />
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-visible", "sk-window-event-hide", "sk-window-event-open"],
  },
  {
    slug: "sk-window-event-hide",
    name: "<SkWindow @hide>",
    kind: "event",
    summary: "Fires when the pop-out is hidden through `visible`.",
    platforms: all(OK),
    platformSince: MAC_LINUX_0_3,
    syntax: '<SkWindow @hide="() => void" />',
    notes: ["It fires when `visible` goes from `true` to `false`.", "In Svelte, it is the `onhide` callback prop."],
    examples: [
      {
        title: "Pause a preview when it is hidden",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

defineProps<{ peeking: boolean }>();
const video = ref<HTMLVideoElement | null>(null);
</script>

<template>
  <SkWindow title="Preview" :visible="peeking" @hide="video?.pause()">
    <video ref="video" src="assets/clip.mp4" />
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-visible", "sk-window-event-show"],
  },
  {
    slug: "sk-window-el",
    name: "<SkWindow> el",
    kind: "exposed",
    summary: "The pop-out's root element, to use as a `<Teleport>` target.",
    platforms: all(OK),
    syntax: "el: Ref<SkWindowTarget | null>",
    notes: ["It is set at mount, before `@open`.", "In Svelte, read it with `bind:el`."],
    examples: [
      {
        title: "Move an existing panel into a pop-out",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, ref } from "vue";
import { SkWindow, type SkWindowTarget } from "@skathi-native/runtime";

const popped = ref(false);
const win = ref<{ el: SkWindowTarget | null } | null>(null);
const target = computed(() => win.value?.el ?? null);
</script>

<template>
  <Teleport :to="target" :disabled="!popped || !target">
    <section class="layers-panel"><slot /></section>
  </Teleport>
  <button @click="popped = !popped">{{ popped ? "Dock" : "Pop out" }}</button>
  <SkWindow v-if="popped" ref="win" title="Layers" @close="popped = false" />
</template>`,
        note: "Write the `<Teleport>` before the `<SkWindow>` it targets.",
      },
    ],
    related: ["sk-window", "sk-window-map-from-main"],
    aliases: ["Teleport"],
  },
  {
    slug: "sk-window-window-id",
    name: "<SkWindow> windowId",
    kind: "exposed",
    summary: "The pop-out's window id, `0` before `@open` and after `@close`.",
    platforms: all(OK),
    syntax: "windowId: Ref<number>",
    notes: ["In Svelte, read it with `bind:windowId`."],
    examples: [
      {
        title: "Show the window id in a debug overlay",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const win = ref<{ windowId: number } | null>(null);
</script>

<template>
  <SkWindow ref="win" title="Inspector"><slot /></SkWindow>
  <small class="debug">inspector window {{ win?.windowId ?? 0 }}</small>
</template>`,
      },
    ],
    related: ["sk-window-event-open"],
  },
  {
    slug: "sk-window-width",
    name: "<SkWindow> width",
    kind: "exposed",
    summary: "The pop-out's content width in CSS px, `0` until it first paints.",
    platforms: all(OK),
    syntax: "width: Ref<number>",
    notes: ["It is updated with each `@resize`.", "In Svelte, read it with `bind:width`."],
    examples: [
      {
        title: "Show the pop-out's size in the main window",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";
${SK_IMPORT}

const win = ref<{ width: number; height: number } | null>(null);
</script>

<template>
  <SkWindow ref="win" title="Canvas"><slot /></SkWindow>
  <span class="size">{{ Math.round(win?.width ?? 0) }} × {{ Math.round(win?.height ?? 0) }}</span>
</template>`,
      },
    ],
    related: ["sk-window-height", "sk-window-event-resize"],
  },
  {
    slug: "sk-window-height",
    name: "<SkWindow> height",
    kind: "exposed",
    summary: "The pop-out's content height in CSS px, `0` until it first paints.",
    platforms: all(OK),
    syntax: "height: Ref<number>",
    notes: ["It is updated with each `@resize`.", "In Svelte, read it with `bind:height`."],
    examples: [
      {
        title: "Fit a list to the pop-out's height",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, ref } from "vue";
${SK_IMPORT}

const win = ref<{ height: number } | null>(null);
const rows = computed(() => Math.max(1, Math.floor((win.value?.height ?? 0) / 28)));
</script>

<template>
  <SkWindow ref="win" title="Recent">
    <ul><li v-for="n in rows" :key="n">Item {{ n }}</li></ul>
  </SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-width", "sk-window-event-resize"],
  },
  {
    slug: "sk-window-scale",
    name: "<SkWindow> scale",
    kind: "exposed",
    summary: "The display scale of the pop-out's monitor, `1` until it first paints.",
    platforms: all(OK),
    syntax: "scale: Ref<number>",
    notes: [
      "It can differ from `devicePixelRatio`, which is the main window's.",
      "In Svelte, read it with `bind:scale`.",
    ],
    examples: [
      {
        title: "Size a canvas for the pop-out's own display",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref, watchEffect } from "vue";
${SK_IMPORT}

const win = ref<{ width: number; height: number; scale: number } | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);

watchEffect(() => {
  if (!win.value || !canvas.value) return;
  canvas.value.width = Math.round(win.value.width * win.value.scale);
  canvas.value.height = Math.round(win.value.height * win.value.scale);
});
</script>

<template>
  <SkWindow ref="win" title="Scope"><canvas ref="canvas" class="scope" /></SkWindow>
</template>`,
      },
    ],
    related: ["sk-window-event-resize", "scale-factor"],
  },
  {
    slug: "sk-window-map-from-main",
    name: "<SkWindow> mapFromMain()",
    kind: "exposed",
    summary: "Returns where an element of the main window is, in the pop-out's own coordinates.",
    platforms: os(OK, OK, NO),
    syntax: "mapFromMain(target: SkWindowTarget | null | undefined): SkWindowRect | null",
    params: [
      { name: "target", type: "SkWindowTarget", description: "An element of the main window, such as a template ref." },
    ],
    returns:
      "`{ x, y, width, height }` in the pop-out's CSS px, relative to its root, or `null` before `@open`, after close, or when either element is not painted.",
    notes: [
      "It maps through screen coordinates, so it works across monitors with different scales. On macOS that is AppKit's screen space, in points on every display.",
      "It answers while the pop-out is hidden. For a `cover` window, it measures where the next show will place the window.",
      "To place a small pop-out at an element, add the result to the `bounds` position the pop-out has now.",
      "In Svelte, `bind:this` on `<SkWindow>` gives an object with this method.",
    ],
    examples: [
      {
        title: "Spotlight a main-window button from a tutorial overlay",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed, ref } from "vue";
import { SkWindow, type SkWindowTarget } from "@skathi-native/runtime";

type Rect = { x: number; y: number; width: number; height: number };

const props = defineProps<{ target: SkWindowTarget | null }>();
const overlay = ref<{ mapFromMain(t: SkWindowTarget | null): Rect | null } | null>(null);
const spot = computed(() => overlay.value?.mapFromMain(props.target) ?? null);
</script>

<template>
  <SkWindow ref="overlay" cover="monitor" backdrop="transparent" ignore-mouse-events :focusable="false">
    <div v-if="spot" class="ring" :style="{ left: spot.x + 'px', top: spot.y + 'px', width: spot.width + 'px', height: spot.height + 'px' }" />
  </SkWindow>
</template>`,
      },
    ],
    limitations: ["On Linux, it always returns `null`."],
    related: ["sk-window-cover", "sk-window-el"],
  },
  {
    slug: "sk-window-creation-time-props",
    name: "SK_WINDOW_CREATION_TIME_PROPS",
    kind: "property",
    summary: "Lists the `<SkWindow>` props that only take effect when the window opens.",
    platforms: all(OK),
    syntax:
      'import { SK_WINDOW_CREATION_TIME_PROPS } from "@skathi-native/runtime";\n\nconst SK_WINDOW_CREATION_TIME_PROPS: readonly string[]; // chrome, resizable, cover, taskbar, focusable, lowLatencySurface',
    notes: [
      "A change of any of them on an open window prints an advisory and does nothing.",
      "Every other prop is live.",
    ],
    examples: [
      {
        title: "Reopen a pop-out when a creation-time prop changes",
        lang: "vue",
        code: `<script setup lang="ts">
import { computed } from "vue";
${SK_IMPORT}

const props = defineProps<{ chrome: "default" | "hidden"; resizable: boolean }>();
const key = computed(() => props.chrome + "|" + props.resizable);
</script>

<template>
  <SkWindow :key="key" title="Preview" :chrome="chrome" :resizable="resizable"><slot /></SkWindow>
</template>`,
        note: "A new `key` unmounts the old component and mounts a new one, which opens a new window.",
      },
    ],
    related: ["sk-window", "sk-window-resizable", "sk-window-chrome"],
  },
  {
    slug: "window-open",
    name: "window.open()",
    kind: "function",
    summary:
      "Opens an `http:`, `https:` or `mailto:` URL with the system handler; it never opens a second window of the app.",
    platforms: all(NO),
    mdn: `${MDN}API/Window/open`,
    syntax: "window.open(url?: string, target?: string, features?: string): null",
    returns: "`null` in every case.",
    notes: [
      "An external URL goes to the system browser or mail client, as with `noopener`.",
      "Any other call (no URL, a relative URL, `about:blank`) does nothing. There is one document; a pop-out shares it.",
      "See [window.open](/web-platform/window-open/) in the web platform reference.",
    ],
    workaround: "Mount a `<SkWindow>` to show part of the app in a second window.",
    related: ["sk-window"],
    aliases: ["popup"],
  },
]);

const taskbar = section("Taskbar and dock", [
  {
    slug: "set-app-badge",
    name: "navigator.setAppBadge()",
    kind: "method",
    summary: "Shows a number or a dot on the app's taskbar button or Dock icon.",
    platforms: os(OK, OK, NO),
    mdn: `${MDN}API/Navigator/setAppBadge`,
    syntax: "navigator.setAppBadge(contents?: number): Promise<void>",
    params: [
      {
        name: "contents",
        type: "number",
        description: "The count. Without it, a dot is shown. `0` clears the badge.",
        optional: true,
      },
    ],
    returns: "A promise that resolves once the badge is set.",
    notes: [
      "The count is truncated to an integer. Counts above 99 show as `99+`.",
      "On Windows, the badge is a red overlay icon on the taskbar button. On macOS, it is the Dock tile's badge.",
      "See [navigator.setAppBadge](/web-platform/navigator-set-app-badge/) in the web platform reference.",
    ],
    examples: [
      {
        title: "Show the unread count on the app icon",
        lang: "ts",
        code: `export function showUnread(count: number) {
  if (!("setAppBadge" in navigator)) return;
  void (count > 0 ? navigator.setAppBadge(count) : navigator.clearAppBadge());
}`,
        note: "The check keeps the code working on Linux, where the method does not exist.",
      },
    ],
    edgeCases: [
      "A negative number, `NaN` or `Infinity` rejects with `TypeError`.",
      "On Windows, the badge goes on the window that owns the current call. Which window that is from a timer or a pop-out's handler is not verified.",
    ],
    limitations: ["On Linux, `navigator.setAppBadge` is `undefined`."],
    related: ["clear-app-badge", "is-minimized"],
    aliases: ["badge", "Badging API", "dock badge"],
  },
  {
    slug: "clear-app-badge",
    name: "navigator.clearAppBadge()",
    kind: "method",
    summary: "Removes the badge from the app's taskbar button or Dock icon.",
    platforms: os(OK, OK, NO),
    mdn: `${MDN}API/Navigator/clearAppBadge`,
    syntax: "navigator.clearAppBadge(): Promise<void>",
    returns: "A promise that resolves once the badge is removed.",
    notes: ["The same as `navigator.setAppBadge(0)`."],
    examples: [
      {
        title: "Clear the badge when the user comes back",
        lang: "ts",
        code: `addEventListener("focus", () => {
  if ("clearAppBadge" in navigator) void navigator.clearAppBadge();
});`,
      },
    ],
    limitations: ["On Linux, `navigator.clearAppBadge` is `undefined`."],
    related: ["set-app-badge", "event-focus"],
  },
  {
    slug: "app-tray",
    name: "appTray",
    kind: "property",
    summary: "Puts an icon with a menu in the system tray or the macOS menu bar.",
    platforms: os(OK, OK, NO),
    syntax:
      'import { appTray } from "skathi:app";\n\nappTray.available: boolean\nappTray.set(options: TrayOptions | null): void',
    params: [
      {
        name: "options.icon",
        type: "string | TrayPixels",
        description: "An asset path, a data URL, or RGBA pixels.",
        optional: true,
      },
      { name: "options.tooltip", type: "string", description: "Text shown on hover.", optional: true },
      {
        name: "options.menu",
        type: "TrayMenuEntry[]",
        description: 'Items with `label` and `onSelect`, `{ type: "separator" }`, submenus, and `{ role: "quit" }`.',
        optional: true,
      },
      {
        name: "options.onClick",
        type: "() => void",
        description: "Runs on a primary click. Without it, the click opens the menu.",
        optional: true,
      },
    ],
    returns: "`set()` returns `undefined`. `null` removes the icon.",
    notes: [
      "`set()` replaces the whole tray model each time.",
      'A `role: "quit"` item quits the app as `appWindow.close()` does: `pagehide` fires, `close-requested` does not.',
      "A window started hidden with no tray icon is shown after 10 s, so the app cannot become unreachable.",
      "Every option, the icon formats and the menu items are in the [App APIs reference](/app-api/app-tray/).",
    ],
    examples: [
      {
        title: "A tray icon that reopens the window",
        lang: "ts",
        code: `import { appTray, appWindow } from "skathi:app";

if (appTray.available) {
  appTray.set({
    icon: "assets/tray.png",
    tooltip: "Sync is running",
    onClick: () => appWindow.show(),
    menu: [
      { label: "Open", onSelect: () => appWindow.show() },
      { type: "separator" },
      { role: "quit", label: "Quit Sync" },
    ],
  });
}`,
      },
    ],
    edgeCases: [
      'An asset path such as `"assets/tray.png"` is read from the packaged app; under `sk dev` it throws a `TypeError`. Use a `data:` URL or pixels there.',
    ],
    limitations: ["On Linux, `available` is `false` and `set()` throws `NotSupportedError`."],
    related: ["show", "hide", "event-close-requested"],
    aliases: ["tray", "system tray", "menu bar extra"],
  },
  {
    slug: "config-windows-icon",
    name: "windows.icon",
    kind: "option",
    summary: "Sets the icon of the Windows exe, which is also the window, taskbar and Alt+Tab icon.",
    platforms: os(OK, OK, NO),
    syntax: '{ "windows": { "icon": string } }',
    params: [
      {
        name: "icon",
        type: "string",
        description: "An `.ico` or a square `.png`, relative to the app root.",
        optional: true,
      },
    ],
    notes: [
      "It is stamped into the exe by `sk build`; it cannot change at run time.",
      "It is also the macOS app icon unless `macos.icon` is set.",
    ],
    examples: [
      {
        title: "One icon for Windows and macOS",
        lang: "ts",
        code: `{
  "name": "notes",
  "windows": { "icon": "assets/icon.png" }
}`,
        note: "A 1024 px PNG stays sharp in the Dock and Finder.",
      },
    ],
    limitations: ["On Linux, the window's icon comes from the `.desktop` entry that matches the app's `id`."],
    related: ["config-macos-icon", "config-id"],
  },
  {
    slug: "config-macos-icon",
    name: "macos.icon",
    kind: "option",
    summary: "Sets the macOS app bundle's icon, shown in the Dock and Finder.",
    platforms: os(NO, OK, NO),
    syntax: '{ "macos": { "icon": string } }',
    params: [
      {
        name: "icon",
        type: "string",
        description: "A square `.png` or an `.icns`, relative to the app root.",
        default: "`windows.icon`",
        optional: true,
      },
    ],
    notes: ["It is written into the `.app` bundle at build time."],
    examples: [
      {
        title: "A macOS-specific icon",
        lang: "ts",
        code: `{
  "windows": { "icon": "assets/icon.ico" },
  "macos": { "icon": "assets/icon-mac.png" }
}`,
      },
    ],
    limitations: ["It applies to macOS only; Windows uses `windows.icon`."],
    related: ["config-windows-icon"],
  },
  {
    slug: "config-id",
    name: "id",
    kind: "option",
    summary: "The app's launch identity, which also ties the window to its Linux desktop entry.",
    platforms: os(PART, PART, OK),
    syntax: '{ "id": string }',
    params: [
      {
        name: "id",
        type: "string",
        description: 'Reverse-DNS, such as `"com.example.notes"`.',
        default: "`name`",
        optional: true,
      },
    ],
    notes: [
      "It names the single-instance lock and the URL scheme registration.",
      "On Linux, it is also the Wayland app id and the X11 class of the main window, which the desktop matches to the `.desktop` entry for the icon and the dock slot.",
    ],
    examples: [
      {
        title: "Give the app a stable identity",
        lang: "ts",
        code: `{
  "name": "notes",
  "id": "com.example.notes",
  "singleInstance": true
}`,
      },
    ],
    edgeCases: ["Pop-outs get no app id on Linux, so the desktop may group them apart from the main window."],
    limitations: ["On Windows and macOS, it is only the launch identity; the window does not carry it."],
    related: ["config-single-instance", "config-windows-icon"],
    aliases: ["app id", "WM_CLASS"],
  },
]);

export const entries: readonly WindowEntry[] = [
  ...mainWindow,
  ...windowEvents,
  ...startup,
  ...titleBar,
  ...popouts,
  ...taskbar,
];
