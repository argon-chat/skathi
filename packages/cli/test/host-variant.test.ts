// Host variants: a prod host is `standard` or `no-devtools`, both with the whole engine, or `aot-nojit` without the JIT. A variant is the name a host
// reports in `--sk-host-info` and the key `sk build` finds it by, so the CLI and crates/sk-app/src/payload.rs must
// agree on the engine's features; the last test reads payload.rs to hold the two lists together.

import { afterAll, describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { modeVariant } from "../cli/build-app";
import { ConfigError, ENGINE_FEATURES, loadAppConfig, parseVariant } from "../cli/config";
import { checkHost, CLI_VERSION, type HostInfo } from "../cli/host";

const roots: string[] = [];
afterAll(() => roots.forEach((r) => rmSync(r, { recursive: true, force: true })));

function app(body: Record<string, unknown>): string {
  const root = mkdtempSync(join(tmpdir(), "sk-variant-"));
  roots.push(root);
  mkdirSync(join(root, "src"));
  writeFileSync(join(root, "src", "main.ts"), "");
  writeFileSync(join(root, "sk.config.json"), JSON.stringify({ name: "t", version: "1.0.0", ...body }));
  return root;
}

describe("variant names", () => {
  test("standard, no-devtools and aot-nojit are the only prod variants", () => {
    expect(parseVariant("aot-nojit", "t")).toBe("aot-nojit");
    expect(parseVariant("standard", "t")).toBe("standard");
    expect(parseVariant(" no-devtools ", "t")).toBe("no-devtools");
    expect(() => parseVariant("no-audio", "t")).toThrow(ConfigError);
    expect(() => parseVariant("no-devtools+no-audio", "t")).toThrow(ConfigError);
  });
});

describe("sk.config.json", () => {
  test('"publishDevtools" is a boolean, off by default', () => {
    expect(loadAppConfig(app({})).publishDevtools).toBe(false);
    expect(loadAppConfig(app({ publishDevtools: true })).publishDevtools).toBe(true);
    expect(() => loadAppConfig(app({ publishDevtools: "yes" }))).toThrow(ConfigError);
  });

  test('"publishJit": false publishes AOT builds on aot-nojit and cannot keep devtools', () => {
    const config = loadAppConfig(app({ publishJit: false }));
    expect(modeVariant(config, "release", true)).toBe("aot-nojit");
    expect(modeVariant(config, "release", false)).toBe("no-devtools");
    expect(modeVariant(config, "debug", true)).toBe("standard");
    expect(modeVariant(loadAppConfig(app({})), "release", true)).toBe("no-devtools");
    expect(() => loadAppConfig(app({ publishJit: false, publishDevtools: true }))).toThrow(ConfigError);
  });

  test("an old engine-part selection is an error that points at publishDevtools and the embedding crate", () => {
    for (const windows of [{ without: ["audio"] }, { features: ["audio"] }, { host: "no-devtools" }]) {
      const load = () => loadAppConfig(app({ windows }));
      expect(load).toThrow(ConfigError);
    }
  });
});

test("the engine features are the ones crates/sk-app/src/payload.rs names the host's variant from, after devtools", () => {
  const rs = readFileSync(resolve(import.meta.dir, "../../../crates/sk-app/src/payload.rs"), "utf8");
  const block = rs.slice(rs.indexOf("pub const OPTIONAL"), rs.indexOf("];", rs.indexOf("pub const OPTIONAL")));
  const names = [...block.matchAll(/\("([a-z]+)",/g)].map((m) => m[1]);
  expect(names).toEqual(["devtools", ...ENGINE_FEATURES]);
});

describe("checkHost", () => {
  const info: HostInfo = {
    mode: "prod",
    skathi: CLI_VERSION,
    crate: "0.1.0",
    variant: "standard",
    devtools: true,
    features: ["prod", ...ENGINE_FEATURES],
    jscPin: "test",
    payloadFormat: 1,
    profile: "release",
  };

  test("a host with the whole engine passes", () => {
    expect(checkHost("current.exe", info, "standard", false)).toBeNull();
    expect(checkHost("current.exe", info, null, false)).toBeNull();
  });

  test("an old cached host missing an engine feature is refused", () => {
    const old = { ...info, features: info.features.filter((f) => f !== "webgpu") };
    expect(checkHost("cached.exe", old, null, false)).not.toBeNull();
    expect(checkHost("cached.exe", old, "standard", false)).not.toBeNull();
  });

  test("a trimmed host built from the embedding crate is not one sk build stamps", () => {
    const trimmed = { ...info, variant: "no-audio", features: info.features.filter((f) => f !== "audio") };
    expect(checkHost("trimmed.exe", trimmed, null, false)).not.toBeNull();
  });
});
