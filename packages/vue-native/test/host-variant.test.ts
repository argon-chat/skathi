// Host variants: which engine parts a bare host carries (cli/config.ts `OPTIONAL`). A variant is the name a host
// reports in `--vn-host-info` and the key `vn build` finds it by, so the CLI and crates/vn-app/src/payload.rs must
// spell it the same way; the last test reads payload.rs to hold the two lists together.

import { afterAll, describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  cargoFeatureArgs,
  ConfigError,
  loadAppConfig,
  OPTIONAL,
  parseParts,
  partsLeftOut,
  selectVariant,
  variantFor,
  variantWarnings,
} from "../cli/config";

const roots: string[] = [];
afterAll(() => roots.forEach((r) => rmSync(r, { recursive: true, force: true })));

/** A throwaway app whose vn.config.json has this `windows` block. */
function app(windows: unknown): string {
  const root = mkdtempSync(join(tmpdir(), "vn-variant-"));
  roots.push(root);
  mkdirSync(join(root, "src"));
  writeFileSync(join(root, "src", "main.ts"), "");
  writeFileSync(join(root, "vn.config.json"), JSON.stringify({ name: "t", version: "1.0.0", windows }));
  return root;
}

describe("variant names", () => {
  test("every part in is standard; the pre-features names are unchanged", () => {
    expect(variantFor([])).toBe("standard");
    expect(variantFor(["devtools"])).toBe("no-devtools");
  });

  test("the parts left out, in OPTIONAL's order whatever order they were given in", () => {
    expect(variantFor(["audio"])).toBe("no-audio");
    expect(variantFor(["audio", "devtools"])).toBe("no-devtools+no-audio");
    expect(variantFor(["indexeddb", "clipboard", "audio"])).toBe("no-audio+no-clipboard+no-indexeddb");
  });

  test("a name reads back as its parts, and an unknown one is an error", () => {
    expect(partsLeftOut("standard")).toEqual([]);
    expect(partsLeftOut("no-audio+no-devtools")).toEqual(["devtools", "audio"]);
    expect(() => partsLeftOut("no-video")).toThrow(ConfigError);
    expect(() => partsLeftOut("no-voice")).toThrow(ConfigError);
    expect(() => partsLeftOut("lite")).toThrow(ConfigError);
  });
});

describe("selection", () => {
  test("features names what to take; without names what to leave out", () => {
    const taken = OPTIONAL.filter((p) => p !== "audio");
    expect(selectVariant({ features: [...taken] }, "t")).toBe("no-audio");
    expect(selectVariant({ without: ["audio"] }, "t")).toBe("no-audio");
    expect(selectVariant({ host: "no-devtools" }, "t")).toBe("no-devtools");
    expect(selectVariant({}, "t")).toBe("standard");
  });

  test("two ways of saying it at once is an error, not a guess", () => {
    expect(() => selectVariant({ features: ["audio"], without: ["clipboard"] }, "t")).toThrow(/one of/);
  });

  test("a list is a comma string or an array of known parts", () => {
    expect(parseParts("audio, clipboard", "t")).toEqual(["audio", "clipboard"]);
    expect(parseParts("audio clipboard", "t")).toEqual(["audio", "clipboard"]); // PowerShell's unquoted audio,clipboard
    expect(parseParts(["clipboard", "audio", "clipboard"], "t")).toEqual(["audio", "clipboard"]);
    expect(() => parseParts(["voice"], "t")).toThrow(/not a part/);
    expect(() => parseParts(3, "t")).toThrow(ConfigError);
  });

  test("a host with indexedDB but no storages is warned about", () => {
    expect(variantWarnings("no-audio")).toEqual([]);
    expect(variantWarnings("no-storages")[0]).toMatch(/in memory/);
    expect(variantWarnings("standard")).toEqual([]);
  });
});

describe("cargo", () => {
  test("WebGPU survives other feature opt-outs and has an explicit opt-out", () => {
    expect(cargoFeatureArgs("no-audio")[2].split(",")).toContain("webgpu");
    expect(cargoFeatureArgs("no-webgpu")[2].split(",")).not.toContain("webgpu");
  });

  test("the two pre-features hosts keep the command line they always had", () => {
    expect(cargoFeatureArgs("standard")).toEqual(["--features", "prod"]);
    expect(cargoFeatureArgs("no-devtools")).toEqual(["--features", "prod,no-devtools"]);
  });

  test("leaving out a default feature drops the defaults and names the rest", () => {
    expect(cargoFeatureArgs("no-clipboard")).toEqual([
      "--no-default-features",
      "--features",
      "prod,audio,storages,indexeddb,webgpu",
    ]);
    expect(cargoFeatureArgs("no-devtools+no-audio")).toEqual([
      "--no-default-features",
      "--features",
      "prod,no-devtools,clipboard,storages,indexeddb,webgpu",
    ]);
  });
});

describe("vn.config.json", () => {
  test('"without", "features" and "host" each select a variant', () => {
    expect(loadAppConfig(app({ without: ["audio"] })).windows.variant).toBe("no-audio");
    expect(loadAppConfig(app({ features: ["audio", "storages"] })).windows.variant).toBe(
      "no-devtools+no-clipboard+no-indexeddb+no-webgpu",
    );
    expect(loadAppConfig(app({ host: "no-devtools" })).windows.variant).toBe("no-devtools");
    expect(loadAppConfig(app({})).windows.variant).toBe("standard");
  });

  test("an unknown part, a bad host name, or two selections are build errors", () => {
    expect(() => loadAppConfig(app({ without: ["video"] }))).toThrow(/not a part/);
    expect(() => loadAppConfig(app({ host: "lite" }))).toThrow(/not a host variant/);
    expect(() => loadAppConfig(app({ host: "no-devtools", without: ["audio"] }))).toThrow(/one of/);
  });
});

test("the parts are the ones crates/vn-app/src/payload.rs names the host's variant from, in its order", () => {
  const rs = readFileSync(resolve(import.meta.dir, "../../../crates/vn-app/src/payload.rs"), "utf8");
  const block = rs.slice(rs.indexOf("pub const OPTIONAL"), rs.indexOf("];", rs.indexOf("pub const OPTIONAL")));
  const names = [...block.matchAll(/\("([a-z]+)",/g)].map((m) => m[1]);
  expect(names).toEqual([...OPTIONAL]);
});

test("standard packaging refuses an old cached host without WebGPU", async () => {
  const { checkHost, CLI_VERSION } = await import("../cli/host");
  const info = {
    mode: "prod" as const,
    vueNative: CLI_VERSION,
    crate: "0.1.0",
    variant: "standard",
    devtools: true,
    features: ["prod", "audio", "clipboard", "storages", "indexeddb"],
    jscPin: "test",
    payloadFormat: 1,
    profile: "release",
  };
  expect(checkHost("cached.exe", info, null, false)).toMatch(/lacks native WebGPU/);
  expect(checkHost("cached.exe", info, "standard", false)).toMatch(/lacks native WebGPU/);
  expect(checkHost("current.exe", { ...info, features: [...info.features, "webgpu"] }, "standard", false)).toBeNull();
  expect(checkHost("explicit.exe", { ...info, variant: "no-webgpu" }, "no-webgpu", false)).toBeNull();
});
