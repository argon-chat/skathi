import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { CLI_VERSION } from "../cli/host";
import { SK } from "./check-fixtures";

const scratch = mkdtempSync(join(tmpdir(), "sk-cli-"));
afterAll(() => rmSync(scratch, { recursive: true, force: true }));

function sk(args: string[], env: Record<string, string | undefined> = { NO_COLOR: "1" }) {
  const r = Bun.spawnSync([process.execPath, SK, ...args], {
    cwd: scratch,
    env: { ...process.env, FORCE_COLOR: undefined, NO_COLOR: undefined, SK_DEBUG: undefined, ...env },
  });
  return { status: r.exitCode ?? -1, stdout: r.stdout.toString(), stderr: r.stderr.toString() };
}

describe("help", () => {
  test("the top level lists every command once, and no retired or internal one", () => {
    const r = sk(["--help"]);
    expect(r.status).toBe(0);
    expect(r.stdout.startsWith(`sk ${CLI_VERSION}  Skathi apps for Windows, macOS and Linux\n`)).toBe(true);
    for (const row of ["create [dir]", "dev [app]", "build [app]", "publish [app]", "run [app]", "check [app]"])
      expect(r.stdout).toContain(`  ${row}`);
    expect(r.stdout).toMatch(/^ {2}workload <command> +Install, list and prune/m);
    expect(r.stdout).toMatch(/^ {2}prune +/m);
    for (const gone of [/^ {2}host\b/m, /^ {2}help\b/m, /^ {2}version\b/m, /-h, --help/])
      expect(r.stdout).not.toMatch(gone);
  });

  test("sk <command> --help and sk help <command> are the same generated text", () => {
    const flag = sk(["build", "--help"]);
    expect(flag.status).toBe(0);
    expect(flag.stdout).toBe(sk(["help", "build"]).stdout);
    expect(flag.stdout.split("\n").slice(0, 3)).toEqual([
      "Usage  sk build [app] [options]",
      "",
      "A debug build in <app>/dist/debug: devtools (F12) on, the bundle not minified.",
    ]);
    expect(flag.stdout).toMatch(/\nOptions\n {2}--platform <os> +windows, macos or linux/);
    expect(flag.stdout).toMatch(
      /\nAdvanced\n {2}--workload <exe> +Target this workload when stamping an executable or building a standalone payload/,
    );
    for (const hiddenFlag of ["--host", "--variant", "--no-aot", "--without", "--allow-"])
      expect(flag.stdout).not.toMatch(new RegExp(`^ {2}${hiddenFlag}`, "m"));
    expect(sk(["workload", "prune", "--help"]).stdout).toContain("Usage  sk workload prune [options]");
  });

  test("--color forces colour over NO_COLOR, --no-color turns it off over FORCE_COLOR", () => {
    expect(sk(["--help", "--color"]).stdout).toContain("\x1b[");
    expect(sk(["--help", "--no-color"], { FORCE_COLOR: "1" }).stdout).not.toContain("\x1b[");
    expect(sk(["--help"]).stdout).not.toContain("\x1b[");
  });
});

describe("usage", () => {
  test("-v, --version and sk version print the version", () => {
    for (const args of [["-v"], ["--version"], ["version"]]) expect(sk(args).stdout.trim()).toBe(CLI_VERSION);
  });

  test("an unknown command or option says what to run, and exits 1", () => {
    const command = sk(["biuld"]);
    expect(command.status).toBe(1);
    expect(command.stderr).toContain("Did you mean build?");
    const option = sk(["workload", "prune", "--frob"]);
    expect(option.status).toBe(1);
    expect(sk(["check", "--frob"]).status).toBe(2);
  });

  test("an unexpected error is one line, with its stack under SK_DEBUG", () => {
    const blocker = join(scratch, "a-file");
    writeFileSync(blocker, "");
    const plain = sk(["create", join(blocker, "app"), "-y"]);
    expect(plain.status).toBe(1);
    expect(plain.stderr.trim().split("\n")).toHaveLength(1);
    expect(plain.stderr).toStartWith("error: ");
    const debug = sk(["create", join(blocker, "app"), "-y"], { NO_COLOR: "1", SK_DEBUG: "1" });
    expect(debug.status).toBe(1);
    expect(debug.stderr).toMatch(/\n\s+at /);
  });
});

describe("sk workload list and sk prune", () => {
  const local = join(scratch, "local");
  const env = { NO_COLOR: "1", LOCALAPPDATA: local, XDG_CACHE_HOME: local, SK_AOT_CACHE_DIR: join(local, "images") };
  const exe = process.platform === "win32" ? "sk-host.exe" : "sk-host";
  const hosts =
    process.platform === "darwin"
      ? join(process.env.HOME ?? "", "Library", "Caches", "skathi", "hosts")
      : join(local, "skathi", "hosts");

  test.skipIf(process.platform === "darwin")("list the cache, then prune it", () => {
    for (const [version, variant] of [
      [CLI_VERSION, "standard"],
      ["0.0.1", "standard"],
    ] as const) {
      mkdirSync(join(hosts, version, variant), { recursive: true });
      writeFileSync(join(hosts, version, variant, exe), new Uint8Array(2048));
    }
    const list = sk(["workload", "list", "--json"], env);
    expect(list.status).toBe(0);
    const json = JSON.parse(list.stdout) as { root: string; entries: { version: string; needed: boolean }[] };
    expect(json.root).toBe(hosts);
    expect(json.entries.map((e) => [e.version, e.needed])).toEqual([
      [CLI_VERSION, true],
      ["0.0.1", false],
    ]);
    const dry = sk(["prune", "--dry-run"], env);
    expect(dry.stdout).toMatch(/^ {2}0\.0\.1 +standard workload +0\.0 MiB +would remove$/m);
    expect(sk(["workload", "list"], env).stdout).toContain("0.0.1");
    const pruned = sk(["workload", "prune"], env);
    expect(pruned.status).toBe(0);
    expect(pruned.stdout).toMatch(/^ {2}0\.0\.1 +standard workload +0\.0 MiB +removed$/m);
  });
});
