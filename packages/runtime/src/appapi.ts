// The app APIs the web lacks: `appPermissions`, `appStorage`, `appShell`, `appCapture`, `appLaunch`.
// The host crates install them natively; a host without one gets an inert stand-in that says so once.

import { warnCompat } from "./compat";

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
  readonly source: "second-instance" | "open-url" | "reopen";
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

const host = globalThis as {
  __SK_PERMISSIONS__?: AppPermissions;
  __SK_STORAGE__?: AppStorage;
  __SK_SHELL__?: ShellHost;
  __SK_CAPTURE__?: AppCapture;
  __SK_LAUNCH__?: AppLaunch;
};

const PERMISSION_NAMES: SkPermissionName[] = [
  "microphone",
  "camera",
  "display-capture",
  "notifications",
  "clipboard-read",
  "clipboard-write",
  "geolocation",
];

function inert<T>(key: string, why: string, members: Record<string, unknown>, fallback: (name: string) => unknown): T {
  return new Proxy(
    {},
    {
      get(_t, name) {
        warnCompat(key, why);
        const k = String(name);
        return Object.prototype.hasOwnProperty.call(members, k) ? members[k] : fallback(k);
      },
    },
  ) as T;
}

function noDesktop(name: string): AppDesktopError {
  const e: AppDesktopError = new Error(`${name}: no desktop host`);
  e.name = "NotSupportedError";
  e.code = "ENOSYS";
  return e;
}

export const appPermissions: AppPermissions =
  host.__SK_PERMISSIONS__ ??
  inert<AppPermissions>(
    "appPermissions",
    "the permissions host is missing (sk_permissions::install did not run) — `appPermissions` is inert and every capability reads as 'unsupported'.",
    {
      platform: "this platform",
      names: PERMISSION_NAMES.slice(),
      query: () => "unsupported",
      list: () => PERMISSION_NAMES.map((name) => ({ name, state: "unsupported", canRequest: false })),
      request: () => Promise.resolve("unsupported"),
    },
    () => () => undefined,
  );

export const appStorage: AppStorage =
  host.__SK_STORAGE__ ??
  inert<AppStorage>(
    "appStorage",
    "the storage host is missing (sk_storage::install did not run, or the host was built without the `storages` feature) — `appStorage` is inert and localStorage does not persist.",
    {
      kinds: [],
      folder: "",
      encrypted: false,
      info: () => ({
        folder: "",
        file: "",
        encrypted: false,
        bytes: 0,
        loaded: false,
        pending: false,
        provider: "",
        kinds: [],
        error: "no storage host",
        indexedDB: null,
      }),
      flush: () => false,
    },
    // A wrong path is worse than none: the app would write somewhere real.
    (name) => () => {
      throw new TypeError(`appStorage.${name} is unavailable: no storage host`);
    },
  );

export const shellHost: ShellHost | null = host.__SK_SHELL__ ?? null;

export const appShell: AppShell =
  shellHost ??
  inert<AppShell>(
    "appShell",
    "the desktop host is missing (sk_desktop::install did not run) — `appShell` is inert and every call rejects.",
    {},
    (name) => () => Promise.reject(noDesktop(`appShell.${name}`)),
  );

export const appCapture: AppCapture =
  host.__SK_CAPTURE__ ??
  inert<AppCapture>(
    "appCapture",
    "the desktop host is missing (sk_desktop::install did not run) — `appCapture` has no sources.",
    {
      permission: () => ({ state: "unsupported", canRequest: false, deniedBySystem: false }),
      requestPermission: () => Promise.resolve("unsupported"),
      permissionHint: () => "",
      getSources: () => Promise.reject(noDesktop("appCapture.getSources")),
      setPendingSource: () => false,
    },
    () => () => undefined,
  );

export const appLaunch: AppLaunch =
  host.__SK_LAUNCH__ ??
  inert<AppLaunch>(
    "appLaunch",
    "the desktop host is missing (sk_desktop::install did not run) — `appLaunch` is empty.",
    {
      argv: Object.freeze([]),
      cwd: "",
      url: null,
      paths: Object.freeze([]),
      singleInstance: false,
      registerProtocol: () => {
        throw new TypeError("appLaunch.registerProtocol is unavailable: no desktop host");
      },
      unregisterProtocol: () => false,
      isProtocolRegistered: () => false,
      addEventListener: () => undefined,
      removeEventListener: () => undefined,
      dispatchEvent: () => true,
    },
    () => () => undefined,
  );
