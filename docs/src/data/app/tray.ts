import { NO, OK, importLine, os, section } from "./shared";

const IMPORT = importLine("appTray");
const PLATFORMS = os(OK, OK, NO);
const LINUX =
  "On Linux, `appTray.available` is `false` and `set(options)` throws an `Error` named `NotSupportedError`.";

export const tray = section("Tray", [
  {
    slug: "app-tray",
    name: "appTray",
    kind: "property",
    summary: "Puts one icon with a menu in the Windows notification area or the macOS menu bar.",
    platforms: PLATFORMS,
    syntax: `${IMPORT}\n\nappTray.available: boolean\nappTray.set(options: TrayOptions | null): void`,
    notes: [
      "An app has one tray icon. `set()` replaces its whole model (icon, tooltip, menu, click handler); `set(null)` removes it.",
      "Callbacks (`onClick`, a menu item's `onSelect`) run on the JS thread in a later frame, never inside the OS callback. A callback from a model that `set()` has since replaced does not run.",
      "A callback that throws is logged as an error and does not reach the app.",
      "A full reload removes the icon, so the code that creates it runs again. Quitting removes it.",
      "A window started hidden with no tray icon is shown after 10 s, so the app cannot become unreachable.",
      "It is not reactive in Vue or Svelte.",
    ],
    examples: [
      {
        title: "Keep a sync app in the tray",
        lang: "ts",
        code: `import { appTray, appWindow } from "@skathi-native/runtime";

let paused = false;

function renderTray(): void {
  appTray.set({
    tooltip: paused ? "Sync is paused" : "Sync is running",
    onClick: () => appWindow.show(),
    menu: [
      { label: "Open", default: true, onSelect: () => appWindow.show() },
      {
        label: "Pause sync",
        checked: paused,
        onSelect: () => {
          paused = !paused;
          renderTray();
        },
      },
      { type: "separator" },
      { role: "quit", label: "Quit Sync" },
    ],
  });
}

if (appTray.available) renderTray();`,
        note: "Without `icon`, the tray shows the app's own icon. `checked` does not toggle by itself: the item calls `set()` again with the new state.",
      },
    ],
    limitations: [LINUX],
    related: ["tray-set", "tray-icon", "tray-menu", "tray-available"],
    aliases: ["tray", "system tray", "notification area", "menu bar extra", "status item", "Tray"],
  },
  {
    slug: "tray-available",
    name: "appTray.available",
    kind: "property",
    summary: "Whether this platform has a tray: `true` on Windows and macOS, `false` on Linux.",
    platforms: PLATFORMS,
    syntax: "readonly appTray.available: boolean",
    notes: [
      "A constant for the build: it does not say whether an icon is shown, or whether the shell has a notification area right now.",
      "On Windows, an icon set before Explorer is up, or lost when Explorer restarts, is added again once the taskbar is back.",
    ],
    examples: [
      {
        title: "Offer hide-to-tray only where a tray exists",
        lang: "ts",
        code: `import { appTray, appWindow } from "@skathi-native/runtime";

appWindow.addEventListener("close-requested", (event) => {
  if (!appTray.available) return;
  event.preventDefault();
  appWindow.hide();
});`,
        note: "On Linux the close goes ahead, so the user is never left with a hidden window and no icon to bring it back.",
      },
    ],
    related: ["app-tray", "tray-set"],
  },
  {
    slug: "tray-set",
    name: "appTray.set()",
    kind: "method",
    summary: "Shows, replaces or removes the tray icon, its tooltip, its menu and its click handler.",
    platforms: PLATFORMS,
    syntax: "appTray.set(options: TrayOptions | null): void",
    params: [
      {
        name: "options.icon",
        type: "string | TrayPixels",
        description:
          "A packaged asset path, a `data:` URL, or RGBA pixels. Without it, the app's icon. See [TrayOptions.icon](/app-api/tray-icon/).",
        optional: true,
      },
      {
        name: "options.tooltip",
        type: "string",
        description: "Text on hover, at most 127 UTF-16 code units. On macOS an empty tooltip shows the app name.",
        optional: true,
      },
      {
        name: "options.menu",
        type: "TrayMenuEntry[]",
        description:
          "The menu. `false`, `null` and `undefined` entries are skipped. See [TrayMenuItem](/app-api/tray-menu/).",
        optional: true,
      },
      {
        name: "options.onClick",
        type: "() => void",
        description: "Runs on a primary click. Without it, a primary click opens the menu.",
        optional: true,
      },
    ],
    returns: "`undefined`. `set(null)` removes the icon.",
    notes: [
      "Options are read and checked synchronously: an invalid model throws and the icon shown before stays as it was.",
      "The native update is queued, and several `set()` calls in a row show only the last.",
      "A primary click (left click, or Enter on the focused icon on Windows) runs `onClick`, or opens the menu when there is no `onClick`. A secondary click (right click; on macOS also Control-click) always opens the menu. With neither a menu nor `onClick`, a click does nothing.",
      "The icon decoded from a string is cached: calling `set()` again with the same string does not decode it again.",
    ],
    examples: [
      {
        title: "Show unread state in the tooltip",
        lang: "ts",
        code: `import { appTray, appWindow } from "@skathi-native/runtime";

export function showUnread(count: number): void {
  if (!appTray.available) return;
  appTray.set({
    tooltip: count ? \`\${count} unread\` : "No new messages",
    onClick: () => appWindow.show(),
    menu: [{ label: "Open", onSelect: () => appWindow.show() }, { role: "quit" }],
  });
}`,
      },
    ],
    edgeCases: [
      "`set(null)` on Linux does nothing and does not throw.",
      "Reading the options runs their getters and proxies once; a Vue `reactive()` model works.",
    ],
    limitations: [
      "A `TypeError` names what is wrong: an option of the wrong type, an empty or over-long label, a tooltip over 127 UTF-16 units, more than 512 menu items, menus nested deeper than 8, or an icon that cannot be read.",
      LINUX,
    ],
    related: ["app-tray", "tray-icon", "tray-menu"],
    aliases: ["TrayOptions"],
  },
  {
    slug: "tray-icon",
    name: "TrayOptions.icon",
    kind: "interface",
    summary: "The tray image: a packaged asset, a `data:` URL, or straight RGBA pixels drawn by the app.",
    platforms: PLATFORMS,
    syntax: `icon?: string | TrayPixels

interface TrayPixels {
  readonly width: number; // 1 to 512
  readonly height: number; // 1 to 512
  readonly data: Uint8Array | Uint8ClampedArray; // width * height * 4 bytes, straight RGBA
  readonly template?: boolean;
}`,
    notes: [
      "A string that is not a `data:` URL names an asset inside the app's package, relative to its root (`assets/tray.png`); a query or fragment is ignored. It is read synchronously from the package, never from disk or the network.",
      "PNG, JPEG and SVG work. The source is fitted into a 128 px square, then into the size the platform draws: the small-icon size for the display scale on Windows, an 18 pt square at 1x and 2x on macOS.",
      "An SVG (a path ending in `.svg`, or a `data:image/svg+xml` URL) is a template icon. `template: true` does the same for pixels: only the alpha channel is used, drawn in the taskbar's foreground colour on Windows and as a template image in the macOS menu bar, so it follows light and dark.",
      "On macOS, a one-tone grey icon is drawn as a template even without `template`, since it would otherwise vanish on one of the two menu bar appearances.",
      "Pixels are copied during `set()`; the array can be reused afterwards.",
      "On Windows, the icon is redrawn when the theme or the display scale changes.",
    ],
    examples: [
      {
        title: "Draw a status dot in code",
        lang: "ts",
        code: `import { appTray, type TrayPixels } from "@skathi-native/runtime";

function dot(rgb: [number, number, number], size = 32): TrayPixels {
  const data = new Uint8ClampedArray(size * size * 4);
  const r = size / 2;
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) {
      const inside = (x + 0.5 - r) ** 2 + (y + 0.5 - r) ** 2 <= (r - 2) ** 2;
      data.set([...rgb, inside ? 255 : 0], (y * size + x) * 4);
    }
  return { width: size, height: size, data };
}

export function setOnline(online: boolean): void {
  appTray.set({ icon: dot(online ? [46, 160, 67] : [140, 140, 140]), tooltip: online ? "Online" : "Offline" });
}`,
      },
      {
        title: "A monochrome glyph that follows the theme",
        lang: "ts",
        code: `import { appTray } from "@skathi-native/runtime";

const glyph =
  "data:image/svg+xml," +
  encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6"/></svg>');

appTray.set({ icon: glyph, tooltip: "Recorder" });`,
        note: "A `data:` URL also works under `sk dev`, where there is no package to read an asset path from.",
      },
    ],
    edgeCases: [
      "Under `sk dev` the app is not packaged, so an asset path throws a `TypeError` saying the asset is not packaged. Use a `data:` URL or pixels there, or build the app.",
      "A string longer than 3 MiB, an image over 2 MiB, or a raster wider or taller than 4096 px throws a `TypeError`.",
      "`data` must be exactly `width * height * 4` bytes and not a view of a `SharedArrayBuffer`; a `Uint8Array` view at an offset into a larger buffer is fine.",
    ],
    limitations: [LINUX],
    related: ["tray-set", "app-tray"],
    aliases: ["TrayPixels", "template image", "tray icon"],
  },
  {
    slug: "tray-menu",
    name: "TrayMenuItem",
    kind: "interface",
    summary: "One entry of the tray menu: an action, a separator, a submenu, or the Quit item.",
    platforms: PLATFORMS,
    syntax: `type TrayMenuEntry = TrayMenuItem | false | null | undefined;

type TrayMenuItem =
  | { type: "separator" }
  | { label: string; disabled?: boolean; checked?: boolean; default?: boolean; onSelect?: () => void }
  | { label: string; disabled?: boolean; submenu: TrayMenuEntry[] }
  | { role: "quit"; label?: string; disabled?: boolean };`,
    notes: [
      "`false`, `null` and `undefined` entries are skipped, so `cond && { … }` works inside the array.",
      "`checked: true` shows a check mark. Selecting the item does not toggle it; set the new state with another `set()`.",
      "`default: true` shows the item in bold on Windows, as the action a primary click would take. macOS ignores it.",
      "`disabled: true` greys the item out; its `onSelect` does not run.",
      'The `role: "quit"` item is labelled "Quit" unless `label` says otherwise. Selecting it quits the app as [`appWindow.close()`](/window-api/close/) does: `pagehide` fires, `close-requested` does not.',
      "On Windows, `&` in a label is shown as is, not as a keyboard mnemonic.",
    ],
    examples: [
      {
        title: "A status submenu and a conditional item",
        lang: "ts",
        code: `import { appTray, type TrayMenuEntry } from "@skathi-native/runtime";

type Status = "online" | "away" | "busy";

export function showTray(status: Status, setStatus: (s: Status) => void, update?: () => void): void {
  const choice = (s: Status, label: string) => ({ label, checked: status === s, onSelect: () => setStatus(s) });
  const menu: TrayMenuEntry[] = [
    { label: "Status", submenu: [choice("online", "Online"), choice("away", "Away"), choice("busy", "Do not disturb")] },
    update && { label: "Restart to update", onSelect: update },
    { type: "separator" },
    { role: "quit" },
  ];
  appTray.set({ menu });
}`,
      },
    ],
    edgeCases: [
      "A quit item cannot also have `onSelect`, `submenu` or `checked`, and a submenu cannot have `onSelect` or `checked`: either throws a `TypeError`.",
      'Labels must be non-empty, without NUL, and at most 512 UTF-16 code units. A `{ type: … }` other than `"separator"` throws.',
      "At most 512 items in the whole menu, nested at most 8 deep.",
      "On Windows, when the user signs out or shuts down, the quit path runs as if the Quit item had been selected.",
    ],
    limitations: [LINUX],
    related: ["tray-set", "app-tray"],
    aliases: ["TrayMenuEntry", "tray menu", "context menu", "quit role"],
  },
]);
