// The runtime without a framework: the DOM surface over the __sk_* host protocol, the document root and the app*
// APIs. An app without Vue (Svelte, hand-written DOM code) gets this as "@skathi-native/runtime"; runtime.ts adds
// Vue's renderer on top. Nothing here may import Vue: such an app has none installed.

import "./apphost";
import type { MirrorElement } from "./runtime/types";
import { ensureDocumentRoot, teardownDocumentRoot } from "./runtime/root";
import { nodeOps } from "./runtime/node-ops";
import { detachNode, looseCreated } from "./runtime/nodes";
import { patchProp } from "./runtime/patch-prop";
import { installStylesheet } from "./runtime/stylesheet";
import { watchDomWrites } from "./runtime/perf-advice";
import "./runtime/reflect";
import "./runtime/dnd";
import "./runtime/document";

export { appRoot, ensureDocumentRoot } from "./runtime/root";
/** @internal The Svelte renderer shares native node registries with the host's runtime module. */
export const svelteNodeOps = { nodeOps, detachNode, looseCreated, patchProp };
export { appCapture, appLaunch, appPermissions, appShell, appStorage } from "./appapi";
export type { AppDesktopError, CapturePermission, CaptureSource, ScreenCaptureSource, SkLaunchEvent } from "./appapi";
export { appWindow, defineWindow, getCurrentWindow, trackWindowReads } from "./runtime/window";
export { appDevtools } from "./devtools";
export type { DevtoolsInfo } from "./devtools";

// Before any app code, so module-scope theme code (`documentElement.classList.add("dark")`) writes to the real root.
ensureDocumentRoot();

/**
 * Mounts an app that renders through the DOM rather than through Vue (Svelte, a hand-written one) on the document root:
 * `render` draws into the root and returns the teardown a reload runs before the next app mounts.
 */
export function mountDocument(render: (root: MirrorElement) => () => void): MirrorElement {
  if (globalThis.__SK_CSS__?.length) installStylesheet(globalThis.__SK_CSS__.join("\n"));
  if (__DEV__) watchDomWrites();
  const root = ensureDocumentRoot();
  const stop = render(root);
  globalThis.__SK_TEARDOWN__ = () => teardownDocumentRoot(root, stop);
  return root;
}

// `<SkWindow>`'s host side; the components are `@skathi-native/runtime`'s (Vue) and `@skathi-native/runtime/svelte`'s.
export { SK_WINDOW_CREATION_TIME_PROPS, createSkWindowPopout, registerWindowRoot } from "./skwindow";
export type {
  SkWindowBounds,
  SkWindowEvents,
  SkWindowMetrics,
  SkWindowOptions,
  SkWindowPopout,
  SkWindowRect,
  SkWindowShown,
  SkWindowState,
  SkWindowTarget,
} from "./skwindow";

// `appHost`: functions and events of a program embedding Skathi as a library; inert in the exe.
export { appHost } from "./apphost";
export { appTray } from "./tray";
export type { AppTray, TrayOptions, TrayPixels, TrayMenuItem, TrayMenuEntry } from "./tray";
export { appHaptics } from "./haptics";
export type { AppHaptics, HapticOptions, HapticPattern, HapticTrigger } from "./haptics";
export { appFiles, type AppFiles } from "./files";
export { appHotkeys } from "./hotkeys";
export type { AppHotkeys, HotkeyBinding, HotkeyButton, HotkeyChord, HotkeyEvent, HotkeyStatus } from "./hotkeys";
export { appLocale } from "./locale";
export type { AppLocale, MessageArgs, Messages } from "./locale";
export type { AppHost, AppHostError } from "./apphost";
