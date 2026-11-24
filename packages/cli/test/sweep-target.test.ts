import { expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

test("sweep removes stale Cargo units and preserves a recently rebuilt sibling", () => {
  const root = mkdtempSync(join(tmpdir(), "skathi-sweep-"));
  try {
    const legacy = join(root, "debug", "deps", "old.rlib");
    const legacyUnit = join(root, "debug", "build", "example-0123456789abcdef");
    const profile = join(root, "x86_64-pc-windows-msvc", "release");
    const pkg = join(profile, "build", "example");
    const stale = join(pkg, "0000000000000001");
    const fresh = join(pkg, "0000000000000002", "fingerprint", "lib-example");
    const exe = join(profile, "example.exe");
    for (const file of [
      join(root, "CACHEDIR.TAG"),
      legacy,
      join(legacyUnit, "out", "old.dll"),
      join(profile, ".cargo-build-lock"),
      join(stale, "out", "old.rlib"),
      fresh,
      exe,
    ]) {
      mkdirSync(dirname(file), { recursive: true });
      writeFileSync(file, "");
    }
    const old = new Date(Date.now() - 30 * 86_400_000);
    for (const entry of readdirSync(root, { withFileTypes: true, recursive: true })) {
      utimesSync(join(entry.parentPath, entry.name), old, old);
    }
    utimesSync(fresh, new Date(), new Date());

    const args = [resolve(import.meta.dir, "../../../scripts/sweep-target.ts"), "--target", root];
    execFileSync(process.execPath, [...args, "--dry-run"]);
    expect(existsSync(stale)).toBe(true);
    expect(existsSync(legacy)).toBe(true);
    execFileSync(process.execPath, args);
    expect(existsSync(stale)).toBe(false);
    expect(existsSync(legacy)).toBe(false);
    expect(existsSync(legacyUnit)).toBe(false);
    expect(existsSync(fresh)).toBe(true);
    expect(existsSync(exe)).toBe(true);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
