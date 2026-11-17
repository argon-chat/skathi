// The app APIs the web lacks: `appPermissions`, `appStorage`, `appShell`, `appCapture`, `appLaunch`.
// The host crates install them natively; a host without one gets an inert stand-in that says so once.

export interface PermissionRow {
  name: string;
  state: SkPermissionState;
  canRequest: boolean;
  deniedBySystem?: boolean;
}

/** `navigator.permissions` plus a standalone `request` and the fourth state, `unsupported`. */
export interface AppPermissions extends EventTarget {
  readonly platform: string;
  readonly names: string[];
  query(name: string): SkPermissionState;
  list(): PermissionRow[];
  /** Asks for a permission and resolves with a fresh OS read, never optimistically (Windows: opens Settings, resolves when the toggle moves or on timeout). */
  request(name: string): Promise<SkPermissionState>;
}

/** IndexedDB's half of `appStorage.info()`: databases live in `<folder>\indexeddb`. `mode` is null until the first `indexedDB` use, which latches folder and key for the run; `null` as a whole without on-disk IndexedDB. */
export interface IndexedDbInfo {
  folder: string;
  started: boolean;
  mode: "plaintext" | "encrypted" | "decrypting" | "refused" | null;
  encrypted: boolean | null;
  key: boolean;
  error: string | null;
}

/** The cookie jar's half of `appStorage.info()`: always sealed; session cookies are never written. `file` is null with no jar backend. */
export interface CookieJarInfo {
  file: string | null;
  sealed: true;
  bytes: number;
  persisted: number;
  pending: boolean;
  error: string | null;
}

export interface StorageInfo {
  folder: string;
  file: string;
  encrypted: boolean;
  bytes: number;
  loaded: boolean;
  pending: boolean;
  provider: string;
  kinds: string[];
  error: string | null;
  indexedDB: IndexedDbInfo | null;
  cookies?: CookieJarInfo;
}

/** `localStorage`'s folder and encryption at rest. Synchronous: boot-time calls with no real wait. */
export interface AppStorage {
  readonly kinds: string[];
  readonly folder: string;
  readonly encrypted: boolean;
  /** Absolute path for a folder kind; creates nothing. Throws TypeError for an unknown kind or a `subfolder` that climbs out. */
  getUserFolder(kind: string, subfolder?: string): string;
  /**
   * Points `localStorage` at `path`, creating it; returns the canonical path.
   * `{ encrypted: true }` encrypts at rest (Windows: DPAPI) and migrates an existing plaintext area;
   * omitting it keeps the current format, and a platform with no provider throws.
   * IndexedDB follows the same folder and encryption but latches both at its first use, so call
   * this before the first `indexedDB` call. The HTTP cache follows `encrypted` but not `path`.
   */
  setStorageFolder(path: string, options?: { encrypted?: boolean }): string;
  info(): StorageInfo;
  flush(): boolean;
}

/** A desktop rejection: `name` is DOMException-style, `code` Node-style, `osError` the raw OS code. */
export interface AppDesktopError extends Error {
  code?: string;
  osError?: number;
}

/**
 * What a browser cannot do. Like Electron's `shell`, with two safer differences: `openPath` refuses
 * programs and scripts unless `{ allowExecutable: true }`, and `openExternal` refuses `file:`.
 * Calls reject with an {@link AppDesktopError} (`ENOENT`, `ENOASSOC`, `EEXECUTABLE`).
 */
export interface AppShell {
  /** Opens `url` with the system handler for its scheme (any but `file:`); resolves once the shell accepts it. */
  openExternal(url: string): Promise<void>;
  /** Opens a file or folder with its default app; `path` must be absolute. */
  openPath(path: string, options?: { allowExecutable?: boolean }): Promise<void>;
  showItemInFolder(path: string): Promise<void>;
  /** Moves `path` to the Recycle Bin, never a permanent delete: rejects (`ENOTRASH`) when there is none. */
  trashItem(path: string): Promise<void>;
}

export interface ScreenCaptureSource {
  id: string;
  name: string;
  thumbnailDataUrl: string;
  appIconDataUrl: string | null;
  displayId: string;
  [key: string]: unknown;
}
export type CaptureSource = "microphone" | "camera" | "screen_share";
export interface CapturePermission {
  state: SkPermissionState;
  canRequest: boolean;
  deniedBySystem: boolean;
}

export interface AppCapture {
  permission(source: CaptureSource): CapturePermission;
  /** Asks for the source: the app's own prompt, or the platform's settings page after a platform denial. */
  requestPermission(source: CaptureSource): Promise<SkPermissionState>;
  permissionHint(source: CaptureSource): string;
  getSources(types?: string[], options?: Record<string, unknown>): Promise<ScreenCaptureSource[]>;
  /** The source the next `getDisplayMedia()` captures, an ID from `getSources()`. */
  setPendingSource(sourceId: string, includeAudio?: boolean): boolean;
}

export interface SkLaunchEvent extends Event {
  readonly argv: readonly string[];
  readonly cwd: string;
  readonly url: string | null;
  readonly paths: readonly string[];
  /** `"open-url"` when the second launch was started for a registered scheme, else `"second-instance"`. */
  readonly source: "second-instance" | "open-url";
}

/** Tier 2 of launching (tier 1 is `launchQueue`): raw argv, cwd, existing-path arguments, the single-instance lock, the `launch` event. Untrusted input. */
export interface AppLaunch extends EventTarget {
  readonly argv: readonly string[];
  readonly cwd: string;
  /** The deep link this process was started for (`myapp://…`), or null. */
  readonly url: string | null;
  readonly paths: readonly string[];
  /** Does THIS process hold the single-instance lock (`"singleInstance": true` and it was first)? */
  readonly singleInstance: boolean;
  /** Registers `scheme://` for this exe, per user. Throws TypeError for an invalid or reserved scheme or one another app owns. */
  registerProtocol(scheme: string): void;
  unregisterProtocol(scheme: string): boolean;
  isProtocolRegistered(scheme: string): boolean;
}

/** `appShell` plus the content-reachable open behind links and `window.open`: http, https and mailto only. */
export interface ShellHost extends AppShell {
  openContent(url: string): Promise<void>;
}

declare const __SK_PERMISSIONS__: AppPermissions;
declare const __SK_STORAGE__: AppStorage;
declare const __SK_SHELL__: ShellHost;
declare const __SK_CAPTURE__: AppCapture;
declare const __SK_LAUNCH__: AppLaunch;

export const appPermissions: AppPermissions = __SK_PERMISSIONS__;
export const appStorage: AppStorage = __SK_STORAGE__;
export const shellHost: ShellHost = __SK_SHELL__;
export const appShell: AppShell = __SK_SHELL__;
export const appCapture: AppCapture = __SK_CAPTURE__;
export const appLaunch: AppLaunch = __SK_LAUNCH__;
