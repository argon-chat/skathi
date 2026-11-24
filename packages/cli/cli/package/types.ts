// The contract between `sk package` and its formats. Each format is `cli/package/<name>.ts` exporting
// `const format: PackageFormat<"<name>">`, registered in `cli/package/index.ts`. Extend this file; do not
// rename what is here, the formats code against it.

import type { AppConfig } from "../config";
import type { Platform } from "../platform";

/** Windows MSIX (#52). */
export interface MsixOptions {
  /** `Identity/@Publisher`: the certificate subject the package is signed with, e.g. `"CN=Example Ltd, O=Example Ltd, C=US"`. */
  publisher: string;
  /** `Identity/@Name`; default: `package.id`. Letters, digits, `.` and `-`, 3 to 50 characters. */
  identityName: string;
  /** `Identity/@ProcessorArchitecture`; default: the build's arch (`x64`, `arm64`). */
  architecture: "x64" | "arm64" | "x86" | "neutral";
  /** `TargetDeviceFamily/@MinVersion`. */
  minVersion: string;
  /** `TargetDeviceFamily/@MaxVersionTested`. */
  maxVersionTested: string;
  /** Capabilities beside `runFullTrust`, e.g. `"internetClient"`, `"microphone"`, `"webcam"`. */
  capabilities: string[];
  /** Tile and splash background: `"transparent"` or `#rrggbb`. */
  background: string;
  /** Write `<name>.appinstaller` pointing at `<url>/<file>.msix`; null: no update feed. */
  appInstaller: AppInstallerOptions | null;
}

export interface AppInstallerOptions {
  /** Where the `.msix` and the `.appinstaller` will be served from (https, no trailing file name). */
  url: string;
  /** Hours between update checks in the background; 0 checks on launch only. */
  hoursBetweenUpdateChecks: number;
  /** Show the update prompt on launch instead of updating silently. */
  showPrompt: boolean;
  /** Users cannot keep running an older version than the feed's. */
  updateBlocksActivation: boolean;
}

/** macOS DMG (#55). Every field optional so `{}` stays valid; `parseOptions` fills them all. */
export interface DmgOptions {
  /** The name of the mounted volume; default: the display name. */
  volumeName?: string;
  /** The image's file name without `.dmg`; default: `<displayName>-<version>-<arch>`. */
  fileName?: string;
  /** `hdiutil convert -format`: LZFSE (`ULFO`, the default), LZMA (`ULMO`), zlib (`UDZO`) or bzip2 (`UDBZ`). */
  format?: DmgImageFormat;
  /** The Finder window: its top-left corner on screen and its size, in points. */
  window?: DmgWindow;
  iconSize?: number;
  textSize?: number;
  /** Icon centres in the window, in points. */
  appPosition?: DmgPoint;
  applicationsPosition?: DmgPoint;
  /** Absolute path of a PNG or TIFF drawn behind the icons; a `<name>@2x.png` beside a PNG is used for Retina. */
  background?: string | null;
  /** `#rrggbb` window colour when there is no background image. */
  backgroundColor?: string | null;
  /** Give the volume the app's icon. */
  volumeIcon?: boolean;
  /** A codesign identity for the image; default: `macos.sign` when it is one; false: leave the image unsigned. */
  sign?: string | false | null;
  /** Notarise and staple the app and the image; null: do not. */
  notarize?: DmgNotarize | null;
}

export type DmgImageFormat = "ULFO" | "ULMO" | "UDZO" | "UDBZ";

export interface DmgPoint {
  x: number;
  y: number;
}

export interface DmgWindow extends DmgPoint {
  width: number;
  height: number;
}

/**
 * `notarytool` credentials: a keychain profile (`xcrun notarytool store-credentials`), or an App Store Connect API
 * key whose `.p8` path or contents is in an environment variable named as `"env:NAME"`. Never a literal key.
 */
export type DmgNotarize =
  { keychainProfile: string; keychain: string | null } | { apiKey: string; apiKeyId: string; apiIssuer: string };

/** Linux AppImage (#56). Every field optional so `{}` stays valid; `parseOptions` fills in the defaults. */
export interface AppImageOptions {
  /** Desktop Menu categories, e.g. `["Office"]`; default `["Utility"]`. */
  categories?: string[];
  /** How the squashfs is compressed; default `"zstd"`. */
  compression?: "zstd" | "gzip";
  /** A local type-2 runtime file instead of the pinned download, for offline builds; null: download. */
  runtime?: string | null;
  /** Embedded update information, e.g. `"gh-releases-zsync|owner|repo|latest|App-*-x86_64.AppImage.zsync"`. */
  updateInformation?: string | null;
  /** The GnuPG key (id, fingerprint or e-mail) whose signature is embedded in the image; null: unsigned. */
  signKey?: string | null;
  /** SPDX expression for the AppStream metainfo's `project_license`. */
  license?: string | null;
  /** The app's web page, for the AppStream metainfo. */
  homepage?: string | null;
  /** Default: `<displayName without spaces>-<version>-<x86_64|aarch64>.AppImage`. */
  fileName?: string | null;
}

/** Options per format, keyed by the name used in `formats` and as the key of the options object. */
export interface FormatOptions {
  msix: MsixOptions;
  dmg: DmgOptions;
  appimage: AppImageOptions;
}

export type FormatName = keyof FormatOptions;

/** The formats a platform section may list; adding a format adds it here. */
export const PLATFORM_FORMATS: { readonly [P in Platform]: readonly FormatName[] } = {
  windows: ["msix"],
  macos: ["dmg"],
  linux: ["appimage"],
};

export interface FileAssociation {
  /** Without the dot, lowercase: `["skt", "skathi-doc"]`. */
  extensions: string[];
  /** Shown as the file type's name, e.g. "Skathi document". */
  name: string;
  /** `application/x-skathi-doc`; null when none was given. */
  mimeType: string | null;
  /** Whether the app opens these files to edit them, only views them, or just lists itself (`none`). */
  role: "editor" | "viewer" | "none";
  /** Absolute path of a square PNG for the file type; null: the app's icon. */
  icon: string | null;
}

export interface UrlProtocol {
  /** Lowercase, without `:` or `//`: `"myapp"` for `myapp://…`. */
  scheme: string;
  /** Shown where the system names the handler. */
  name: string;
}

export interface PackageIdentity {
  /** What the user sees: Start menu, Finder, desktop entry. Default: the app's `productName`. */
  displayName: string;
  /** The person or company shipping the app, as shown to users. */
  publisher: string;
  /** Reverse-DNS, e.g. `com.example.notes`. Default: the app's `"id"` when it is reverse-DNS. */
  id: string;
  /** `major.minor.patch`; default: the app's version. Formats that need four parts append `.0`. */
  version: string;
  /** One line; default: `windows.description`, else the display name. */
  description: string;
  /** Default: `windows.copyright` / `macos.copyright`, else `© <publisher>`. */
  copyright: string;
}

export interface WindowsPackage {
  formats: FormatName[];
  msix: MsixOptions;
}

export interface MacPackage {
  formats: FormatName[];
  dmg: DmgOptions;
}

export interface LinuxPackage {
  formats: FormatName[];
  appimage: AppImageOptions;
}

/** sk.config.json `"package"`, parsed, defaults applied. Paths are absolute. */
export interface PackageConfig {
  identity: PackageIdentity;
  /** Square `.png` files or an `.ico`; empty when neither `package.icon` nor the app's icon is set. Use `PackageContext.icon`. */
  icons: string[];
  fileAssociations: FileAssociation[];
  protocols: UrlProtocol[];
  /** Default: `<app>/dist/package`. */
  outDir: string;
  windows: WindowsPackage;
  macos: MacPackage;
  linux: LinuxPackage;
}

/**
 * Something wrong in the config or the build output; `sk check` reports it under `code`. SK8001 (error: `sk package`
 * refuses to run) and SK8002 (warning) serve every format. A format that needs its own codes takes them from its
 * hundred: msix SK8100–SK8199, dmg SK8200–SK8299, appimage SK8300–SK8399, signing SK8400–SK8499. Every code is
 * registered in check/codes.ts.
 */
export interface Problem {
  code: string;
  severity: "error" | "warning";
  /** The JSON path in sk.config.json, e.g. `package.windows.msix.publisher`. */
  key: string;
  /** What is wrong and what to do. */
  message: string;
}

export interface PackageLog {
  info(line: string): void;
  warn(line: string): void;
}

/** What `validate` sees: the config, before anything is built. */
export interface PackageCheckContext {
  appRoot: string;
  app: AppConfig;
  config: PackageConfig;
  platform: Platform;
}

/** What `write` sees: the config plus the finished `sk build` output. */
export interface PackageContext extends PackageCheckContext {
  /** Node's arch names: `x64`, `arm64`. */
  arch: string;
  /** The folder `sk build` wrote: the exe and what ships beside it. */
  buildDir: string;
  /** The program inside `buildDir` (Windows/Linux), or the executable inside the `.app` (macOS). */
  exe: string;
  /** macOS: the `.app` bundle; null elsewhere. */
  appBundle: string | null;
  /** Created before `write`; formats write their artifacts here and nothing outside it. */
  outDir: string;
  /** Scratch space under `outDir`, removed after the run; formats make their own subfolder. */
  workDir: string;
  log: PackageLog;
  /** A square PNG of `size` px from the configured icons (largest source scaled down); null when there is none. */
  icon(size: number): Uint8Array | null;
  /** Called with each package written before it is listed as an artifact; signing (#53) slots in here. */
  sign: SignHook | null;
}

export type SignHook = (path: string, format: FormatName) => Promise<void>;

export type ArtifactKind =
  /** What a user installs: `.msix`, `.dmg`, `.AppImage`, `.deb`, `.exe` setup. */
  | "installer"
  /** Read by an updater: `.appinstaller`, update manifests, zsync files. */
  | "update"
  /** Anything else worth keeping: checksums, logs, a manifest for inspection. */
  | "other";

export interface Artifact {
  /** Absolute. */
  path: string;
  kind: ArtifactKind;
  format: FormatName;
  /** Whether `sign` ran over it; formats that need no signature say false. */
  signed: boolean;
}

export interface PackageFormat<N extends FormatName = FormatName> {
  name: N;
  platform: Platform;
  /** The systems `sk package` can write it on; left out: `platform` only. Checked before anything is built. */
  hosts?: readonly Platform[];
  /** The keys the options object accepts; any other key is an error with this list in its message. */
  optionKeys: readonly string[];
  /** `package.<platform>.<name>` from sk.config.json (`{}` when absent): options with defaults applied, problems pushed. */
  parseOptions(raw: Record<string, unknown>, ctx: OptionsContext, problems: Problem[]): FormatOptions[N];
  /** Also run by `sk check`; must not need the build output. */
  validate(ctx: PackageCheckContext): Problem[];
  write(ctx: PackageContext): Promise<Artifact[]>;
}

/** What option defaults may derive from, before the rest of the config is assembled. */
export interface OptionsContext {
  app: AppConfig;
  identity: PackageIdentity;
  /** The JSON path of the options object, for `Problem.key`. */
  at: string;
}
