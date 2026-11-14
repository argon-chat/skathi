import { afterAll, afterEach, beforeAll, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cachedHost, type HostInfo } from "../cli/host";
import {
  aotCompilerAssetName,
  cachedAotCompiler,
  fetchAotCompiler,
  fetchHost,
  hostAssetName,
  sha256,
  type HostManifest,
} from "../cli/host-fetch";

const VERSION = "9.9.9-test";
const host = new TextEncoder().encode("MZ pretend host");
const info: HostInfo = {
  mode: "prod",
  skathi: VERSION,
  crate: "0.1.0",
  variant: "standard",
  devtools: true,
  features: ["prod"],
  jscPin: "pin",
  payloadFormat: 1,
  profile: "release",
};
const file = hostAssetName("windows", "x64", "standard");

let served: Record<string, Uint8Array<ArrayBuffer>> = {};
let release: number = 200;
const seen: { path: string; auth: string | null }[] = [];

const server = Bun.serve({
  port: 0,
  fetch(req) {
    const url = new URL(req.url);
    seen.push({ path: url.pathname, auth: req.headers.get("authorization") });
    if (url.pathname === `/repos/o/r/releases/tags/v${VERSION}`) {
      if (release !== 200) return new Response("{}", { status: release });
      const assets = Object.entries(served).map(([name, bytes]) => ({
        name,
        size: bytes.length,
        url: `${url.origin}/assets/${name}`,
      }));
      return Response.json({ assets });
    }
    const name = url.pathname.replace(/^\/assets\//, "");
    const bytes = served[name];
    if (!bytes || req.headers.get("accept") !== "application/octet-stream") return new Response("no", { status: 404 });
    return new Response(bytes);
  },
});

const saved: Record<string, string | undefined> = {};
const ENV = ["SK_HOST_API", "SK_HOST_REPO", "GH_TOKEN", "GITHUB_TOKEN", "LOCALAPPDATA", "XDG_CACHE_HOME"];
let cache: string;

beforeAll(() => {
  for (const k of ENV) saved[k] = process.env[k];
});

afterEach(() => {
  rmSync(cache, { recursive: true, force: true });
  for (const k of ENV) {
    if (saved[k] === undefined) delete process.env[k];
    else process.env[k] = saved[k];
  }
});

afterAll(() => server.stop(true));

function publish(manifest: Partial<HostManifest> = {}, bytes: Uint8Array<ArrayBuffer> = host) {
  cache = mkdtempSync(join(tmpdir(), "sk-host-fetch-"));
  process.env.LOCALAPPDATA = cache;
  process.env.XDG_CACHE_HOME = cache;
  process.env.SK_HOST_API = server.url.href;
  process.env.SK_HOST_REPO = "o/r";
  delete process.env.GH_TOKEN;
  delete process.env.GITHUB_TOKEN;
  release = 200;
  seen.length = 0;
  const m: HostManifest = {
    skathi: VERSION,
    hosts: [
      { platform: "windows", arch: "x64", variant: "standard", file, sha256: sha256(host), size: host.length, info },
    ],
    ...manifest,
  };
  served = {
    "hosts.json": new TextEncoder().encode(JSON.stringify(m)),
    [file]: Bun.gzipSync(bytes),
  };
}

const fetchStandard = () => fetchHost({ variant: "standard", version: VERSION, platform: "windows", arch: "x64" });

test("downloads the host, checks it against hosts.json and caches it with its host.json", async () => {
  publish();
  process.env.GH_TOKEN = "t0ken";
  const { path } = await fetchStandard();
  expect(path).toBe(cachedHost(VERSION, "standard", "windows"));
  expect(new Uint8Array(readFileSync(path))).toEqual(host);
  expect(JSON.parse(readFileSync(join(path, "..", "host.json"), "utf8"))).toEqual(info);
  expect(seen.every((s) => s.auth === "Bearer t0ken")).toBe(true);
});

test("a host that does not match its sha256 is not cached", async () => {
  publish({}, new TextEncoder().encode("MZ something else"));
  await expect(fetchStandard()).rejects.toThrow(/does not match the sha256/);
  expect(existsSync(cachedHost(VERSION, "standard", "windows"))).toBe(false);
});

test("a variant the release does not carry names the ones it does", async () => {
  publish();
  await expect(
    fetchHost({ variant: "no-devtools", version: VERSION, platform: "windows", arch: "x64" }),
  ).rejects.toThrow(/no no-devtools host for windows-x64 \(it has: standard\)/);
});

test("a Linux host is named without .exe and cached under the XDG cache", async () => {
  const elf = new Uint8Array([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1]);
  const linuxFile = hostAssetName("linux", "x64", "standard");
  expect(linuxFile).toBe("sk-host-linux-x64-standard.gz");
  expect(file).toBe("sk-host-windows-x64-standard.exe.gz");
  publish({
    hosts: [
      {
        platform: "linux",
        arch: "x64",
        variant: "standard",
        file: linuxFile,
        sha256: sha256(elf),
        size: elf.length,
        info,
      },
    ],
  });
  served[linuxFile] = Bun.gzipSync(elf);
  const { path } = await fetchHost({ variant: "standard", version: VERSION, platform: "linux", arch: "x64" });
  expect(path).toBe(join(cache, "skathi", "hosts", VERSION, "standard", "sk-host"));
  expect(new Uint8Array(readFileSync(path))).toEqual(elf);
  await expect(fetchStandard()).rejects.toThrow(/no standard host for windows-x64$/);
});

test("a missing release says a private repository needs a token", async () => {
  publish();
  release = 404;
  await expect(fetchStandard()).rejects.toThrow(/has no v9\.9\.9-test release \(a private repository needs GH_TOKEN/);
});

test("the native image compiler comes from the same release, checked and cached with what it compiles for", async () => {
  const compiler = new TextEncoder().encode("MZ pretend compiler");
  const compilerFile = aotCompilerAssetName("windows", "x64");
  expect(compilerFile).toBe("sk-aot-compiler-windows-x64.exe.gz");
  const compiles = { target: "x86_64-windows", frontend: "f", engine: "e", tableId: "t", compileOptions: "o" };
  const entry = {
    platform: "windows" as const,
    arch: "x64",
    file: compilerFile,
    sha256: sha256(compiler),
    size: compiler.length,
  };
  const fetchCompiler = () => fetchAotCompiler({ version: VERSION, platform: "windows", arch: "x64" });

  publish({ aot: [{ ...entry, info: compiles }] });
  served[compilerFile] = Bun.gzipSync(compiler);
  const { path, info: got } = await fetchCompiler();
  expect(path).toBe(cachedAotCompiler(VERSION, "windows"));
  expect(new Uint8Array(readFileSync(path))).toEqual(compiler);
  expect(got).toEqual(compiles);
  expect(JSON.parse(readFileSync(join(path, "..", "compiler.json"), "utf8"))).toEqual(compiles);

  publish({ aot: [{ ...entry, sha256: "0".repeat(64), info: compiles }] });
  served[compilerFile] = Bun.gzipSync(compiler);
  await expect(fetchCompiler()).rejects.toThrow(/does not match the sha256/);
  expect(existsSync(cachedAotCompiler(VERSION, "windows"))).toBe(false);

  publish();
  await expect(fetchCompiler()).rejects.toThrow(/has no native image compiler for windows-x64/);
});
