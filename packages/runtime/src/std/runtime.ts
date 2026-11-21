// `skathi:runtime`: the Skathi this app runs on and how the app reached it. The host installs the objects.

export interface Runtime {
  /** The Skathi version of the host, not the app's own (`app.getVersion()`). */
  readonly version: string;
  /** The optional parts this host was built with: `"webgpu"`, `"audio"`, `"video"`, … */
  readonly features: ReadonlySet<string>;
}

interface RuntimeStatics {
  readonly prototype: Runtime;
  /** The app's code is running from its native image. False after a reload, which runs the bundle. */
  isAot(): boolean;
  /** The app needs no file beside its executable. */
  isSingleFile(): boolean;
}

declare const __sk_std: { Runtime: RuntimeStatics; runtime: Runtime };

export const Runtime: RuntimeStatics = __sk_std.Runtime;
export const runtime: Runtime = __sk_std.runtime;
