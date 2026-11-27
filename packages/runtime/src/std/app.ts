// `skathi:app`: the app's own life (`app`) and the desktop objects around it. The host installs `app`.

export interface App {
  /**
   * Ends the app as `appWindow.close()` does: `pagehide` fires, `close-requested` and `beforeunload`
   * are not asked. `exitCode` (0 to 255, default 0) is the process's exit code.
   */
  quit(options?: { exitCode?: number }): void;
  /**
   * Ends the app and starts it again with the same arguments, or with `args`. Throws
   * `NotSupportedError` when the app runs inside another program.
   */
  restart(options?: { args?: string[] }): void;
  /** `"version"` from sk.config.json, else package.json's; `"0.0.0"` without either. */
  getVersion(): string;
  /**
   * How far a mouse-wheel notch scrolls the app's scroll containers, text areas and editors, as a
   * multiple of a browser's distance; starts at sk.config.json `input.wheelScrollScale`, else 1. Kept
   * between 0.1 and 10; a value that is not a finite number above 0 throws a `TypeError`.
   * `WheelEvent` deltas and touchpad scrolling are not scaled.
   */
  wheelScrollScale: number;
}

declare const __sk_std: { app: App };

export const app: App = __sk_std.app;

export { appCapture, appLaunch, appPermissions, appShell, appStorage } from "../appapi";
export type { AppDesktopError, CapturePermission, CaptureSource, ScreenCaptureSource, SkLaunchEvent } from "../appapi";
export { appWindow } from "../runtime/window";
export { appDevtools } from "../devtools";
export type { DevtoolsInfo } from "../devtools";
export { appHost } from "../apphost";
export type { AppHost, AppHostError } from "../apphost";
export { appTray } from "../tray";
export type { AppTray, TrayOptions, TrayPixels, TrayMenuItem, TrayMenuEntry } from "../tray";
export { appNotifications } from "../notifications";
export type {
  AppNotifications,
  DesktopNotificationOptions,
  NotificationAction,
  NotificationActivation,
  NotificationCapabilities,
  NotificationReceipt,
} from "../notifications";
export { appHaptics } from "../haptics";
export type { AppHaptics, HapticOptions, HapticPattern, HapticTrigger } from "../haptics";
export { appFiles, type AppFiles } from "../files";
export { appHotkeys } from "../hotkeys";
export type { AppHotkeys, HotkeyBinding, HotkeyButton, HotkeyChord, HotkeyEvent, HotkeyStatus } from "../hotkeys";
export { appLocale } from "../locale";
export type { AppLocale, MessageArgs, Messages } from "../locale";
