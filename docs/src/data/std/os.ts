import { importLine, section } from "./shared";

const IMPORT = importLine("skathi:os", "os");

export const os = section("skathi:os", [
  {
    slug: "os",
    name: "os",
    kind: "object",
    summary:
      "Reads the operating system synchronously: platform, architecture, release, host name, home and temp folders.",
    syntax: `${IMPORT}

readonly os.platform: "win32" | "darwin" | "linux"
readonly os.arch: "x64" | "arm64"
readonly os.machineArch: "x64" | "arm64"
readonly os.release: string
readonly os.distro: { readonly name: string; readonly version: string; readonly id: string } | null
readonly os.hostname: string
readonly os.homedir: string
readonly os.tmpdir: string`,
    notes: [
      "The values are synchronous and use Node's names, so code and packages written against `process.platform` and `os.arch()` read the same strings.",
      "`navigator.userAgentData` stays for code written for a browser. Its `getHighEntropyValues()` is asynchronous and reports Chromium's `platformVersion`, not the OS release.",
      '`platform` is `"win32"` on Windows, `"darwin"` on macOS and `"linux"` on Linux.',
      "`arch` is the architecture the host was built for, on every platform.",
      "`machineArch` is the machine's own architecture. On Windows it is the CPU, also for an x64 app on Arm (Prism). On macOS it is the CPU, also under Rosetta. On Linux it is the kernel's machine.",
      '`arch` and `machineArch` differ when the app runs translated: an x64 build on Windows on Arm or under Rosetta reads `arch: "x64"`, `machineArch: "arm64"`.',
      '`release` is `"10.0.26100"` on Windows (major, minor, build), `"15.1"` on macOS (the product version) and the kernel release on Linux, for example `"6.8.0-45-generic"`.',
      "`distro` is the Linux distribution: `NAME`, `VERSION_ID` and `ID` of `/etc/os-release`, or of `/usr/lib/os-release` when the first is missing. The object is frozen. A field the file does not have is an empty string.",
      "`hostname` is the DNS host name on Windows and the result of `gethostname` on macOS and Linux.",
      "`homedir` is the user's profile folder on Windows and `$HOME` on macOS and Linux.",
      "`tmpdir` is the user's temp folder on Windows, `$TMPDIR` on macOS, and `$TMPDIR`, else `/tmp`, on Linux. It has no trailing separator.",
      "The values are read once, on first use, and do not change while the app runs.",
      "The app's own folders are in [`appStorage`](/app-api/app-storage/).",
      "The module also exports `OS`, the class of `os`, for `instanceof` checks. It has no constructor to call.",
    ],
    examples: [
      {
        title: "Pick the download for this machine",
        lang: "ts",
        code: `import { os } from "skathi:os";

const extension = { win32: "exe", darwin: "dmg", linux: "AppImage" } as const;

export function installerName(version: string): string {
  return \`notes-\${version}-\${os.platform}-\${os.machineArch}.\${extension[os.platform]}\`;
}`,
        note: "`machineArch` picks the native build even when the running app is translated.",
      },
      {
        title: "Describe the system in a bug report",
        lang: "ts",
        code: `import { os } from "skathi:os";

export function describeSystem(): string {
  const system = os.distro ? \`\${os.distro.name} \${os.distro.version}\` : os.platform;
  const translated = os.arch === os.machineArch ? "" : \` (\${os.arch} app on \${os.machineArch})\`;
  return \`\${system} \${os.release}\${translated}\`;
}`,
      },
      {
        title: "Shorten a path under the home folder",
        lang: "ts",
        code: `import { os } from "skathi:os";

export function displayPath(path: string): string {
  return os.homedir !== "" && path.startsWith(os.homedir) ? "~" + path.slice(os.homedir.length) : path;
}`,
      },
    ],
    edgeCases: [
      "`distro` is `null` on Windows and macOS. On Linux it is `null` without an os-release file, or when the file has neither `NAME` nor `ID`.",
      "No property throws. `release`, `hostname` and `homedir` are an empty string when the OS does not answer.",
      "`machineArch` equals `arch` when the OS does not answer or reports another architecture.",
      "A `tmpdir` that is a root (`/`, `C:\\`) keeps its separator.",
    ],
    related: ["runtime", "app", "network"],
    aliases: [
      "skathi:os",
      "OS",
      "os.platform",
      "os.arch",
      "os.machineArch",
      "os.release",
      "os.distro",
      "os.hostname",
      "os.homedir",
      "os.tmpdir",
      "process.platform",
      "node:os",
      "Rosetta",
      "Prism",
      "os-release",
      "home directory",
      "temp directory",
    ],
  },
]);
