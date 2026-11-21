// `skathi:os`: the operating system as it is, synchronously. The host installs the object.

export interface OS {
  readonly platform: "win32" | "darwin" | "linux";
  /** The architecture this app's process was built for. */
  readonly arch: "x64" | "arm64";
  /** The machine's own architecture, also when the process runs translated (Prism, Rosetta). */
  readonly machineArch: "x64" | "arm64";
  /** `"10.0.26100"` on Windows, `"15.1"` on macOS, the kernel's release on Linux. */
  readonly release: string;
  /** The Linux distribution from os-release; null elsewhere and when the file is missing. */
  readonly distro: { readonly name: string; readonly version: string; readonly id: string } | null;
  readonly hostname: string;
  readonly homedir: string;
  readonly tmpdir: string;
}

declare const __sk_std: { OS: { readonly prototype: OS }; os: OS };

export const OS: { readonly prototype: OS } = __sk_std.OS;
export const os: OS = __sk_std.os;
