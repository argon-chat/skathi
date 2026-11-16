import type { IconName } from "./icons";

export interface Scenario {
  /** The clip id: docs/public/media/<id>.webm. */
  id: string;
  guide: string;
  title: string;
  icon: IconName;
  lead: string;
  /** The APIs the page uses, shown as chips. */
  apis: string[];
  /** One line on platform reach, when the scenario does not work the same everywhere. */
  platform?: string;
  /** "drawn": the demo animates its own cursor. "real": the recorder moves the mouse. */
  cursor: "drawn" | "real";
}

export const GUIDES = ["Windows", "Custom title bar", "Pop-out windows", "Closing and background"] as const;

export const scenarios: Scenario[] = [
  {
    id: "windows/state-verbs",
    guide: "Windows",
    title: "Minimize, maximize, restore",
    icon: "window",
    lead: "A toolbar drives the window state, and the label follows it on its own: appWindow's is*() reads are reactive, so the computed re-runs whenever the OS changes the window.",
    apis: ["minimize()", "maximize()", "unmaximize()", "toggleMaximize()", "isMaximized()", "isMinimized()"],
    platform:
      "Fully on Windows. On macOS and Linux maximize() toggles, unmaximize() does nothing and isMinimized() stays false.",
    cursor: "drawn",
  },
  {
    id: "windows/always-on-top",
    guide: "Windows",
    title: "Always on top",
    icon: "pin",
    lead: "Shrink the window into a small notes window and pin it. A pinned window stays above other windows, even one that opens or comes to the front after it.",
    apis: ["alwaysOnTop", "setContentBounds()"],
    platform: "Windows only. On macOS and Linux the getter reports the value, but the window is not kept on top.",
    cursor: "drawn",
  },
  {
    id: "windows/theme-backdrop",
    guide: "Windows",
    title: "Theme and backdrop",
    icon: "theme",
    lead: "The theme drives prefers-color-scheme for every window of the app. The backdrop is the material behind the window; it shows wherever the app paints nothing opaque.",
    apis: ["theme", "backdrop", "effectiveBackdrop", "prefers-color-scheme"],
    platform:
      "On Windows the theme also tints Mica and the caption. macOS draws one blur for every material, Linux draws them opaque.",
    cursor: "drawn",
  },
  {
    id: "windows/fullscreen",
    guide: "Windows",
    title: "Fullscreen",
    icon: "fullscreen",
    lead: "Element fullscreen covers the monitor, and Escape leaves it. The title bar hides itself with @media (display-mode: fullscreen), so no script handles the chrome.",
    apis: ["requestFullscreen()", "exitFullscreen()", "isFullscreen()", "display-mode"],
    cursor: "drawn",
  },
  {
    id: "title-bar/drag-region",
    guide: "Custom title bar",
    title: "Drag region",
    icon: "drag",
    lead: "The app paints its own title bar and marks it app-region: drag. Windows treats it as the caption: drag to move, double-click to maximize and restore, drag to the top edge to snap.",
    apis: ["app-region: drag", "app-region: no-drag", "env(titlebar-area-height)"],
    platform: "Windows and Linux. On macOS drag regions are inert; only the system title bar band moves the window.",
    cursor: "real",
  },
  {
    id: "title-bar/overlay",
    guide: "Custom title bar",
    title: "Host caption buttons",
    icon: "tabs",
    lead: "This page swaps the app's own caption buttons for the host-drawn cluster and puts document tabs in the title bar. env(titlebar-area-width) leaves room for the buttons.",
    apis: ["setTitleBarOverlay()", "env(titlebar-area-width)", "chrome.hostControls"],
    platform:
      "Windows: real caption buttons with snap layouts. macOS draws them but they do nothing; on Linux maximize cannot restore.",
    cursor: "real",
  },
  {
    id: "title-bar/edge-resize",
    guide: "Custom title bar",
    title: "Edge resize",
    icon: "resize",
    lead: "A window with a custom title bar keeps its resize edges. Drag the right edge: below 900 px the sidebar collapses to a rail, through a plain @media rule.",
    apis: ["@media (max-width)", "minWidth", "minHeight"],
    cursor: "real",
  },
  {
    id: "pop-out/palette",
    guide: "Pop-out windows",
    title: "Floating palette",
    icon: "palette",
    lead: "A palette in its own window edits the canvas in this one. It is the same component and the same ref, so a click in the palette recolors the canvas with no messaging.",
    apis: ["<SkWindow>", ':taskbar="false"', "@close"],
    platform: "Placement and the owned-window behavior are Windows only; elsewhere the palette opens centered.",
    cursor: "drawn",
  },
  {
    id: "pop-out/follow",
    guide: "Pop-out windows",
    title: "Tooltip that follows",
    icon: "follow",
    lead: "Select a file: its details appear in a click-through overlay window, placed with mapFromMain() against the row. Scroll the list and the tooltip follows the row; scroll it out of view and the tooltip hides.",
    apis: ["mapFromMain()", 'cover="monitor"', "ignoreMouseEvents", ":visible"],
    platform: "Windows only. On macOS and Linux mapFromMain() returns null and cover windows never show.",
    cursor: "drawn",
  },
  {
    id: "pop-out/overlay",
    guide: "Pop-out windows",
    title: "Click-through overlay",
    icon: "overlay",
    lead: "A break timer draws a countdown ring over the desktop. The overlay ignores the mouse and never takes focus, so this window and every other app keep working under it.",
    apis: ['cover="monitor"', 'backdrop="transparent"', "ignoreMouseEvents", ':focusable="false"'],
    platform: "Windows only. On macOS and Linux a cover window is never shown.",
    cursor: "drawn",
  },
  {
    id: "pop-out/reattach",
    guide: "Pop-out windows",
    title: "Close and reattach",
    icon: "reattach",
    lead: "This inspector is a pop-out that is not driven by v-if. Close its window and the subtree comes back inline, here, with its state intact.",
    apis: ["<SkWindow>", "@close"],
    cursor: "drawn",
  },
  {
    id: "closing/unsaved-changes",
    guide: "Closing and background",
    title: "Unsaved changes",
    icon: "unsaved",
    lead: "Edit the note, then press the window's close button. close-requested is cancelled while there is something to save, and the app asks in its own sheet.",
    apis: ["close-requested", "preventDefault()", "close()"],
    platform: "On Linux the custom title bar's close button skips close-requested.",
    cursor: "real",
  },
  {
    id: "closing/hide-and-return",
    guide: "Closing and background",
    title: "Hide and come back",
    icon: "background",
    lead: "With the switch on, the close button hides the window and the app keeps running. Start the app again, or click the tray icon, and the same window comes back.",
    apis: ["close-requested", "hide()", "show()", "appLaunch", "singleInstance", "appTray"],
    platform: "Windows only in full. macOS and Linux minimize instead of hiding, and macOS has no single instance.",
    cursor: "real",
  },
];

export const scenarioById = new Map(scenarios.map((s) => [s.id, s]));
