import { afterAll, afterEach, beforeAll, expect, test } from "bun:test";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cachedHost, type HostInfo } from "../cli/host";
import { fetchHost, hostAssetName, sha256, standIn, type HostManifest } from "../cli/host-fetch";

const VERSION = "9.9.9-test";
const host = new TextEncoder().encode("MZ pretend host");
const info: HostInfo = {
  mode: "prod",
  vueNative: VERSION,
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
const ENV = ["VN_HOST_API", "VN_HOST_REPO", "GH_TOKEN", "GITHUB_TOKEN", "LOCALAPPDATA"];
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
  cache = mkdtempSync(join(tmpdir(), "vn-host-fetch-"));
  process.env.LOCALAPPDATA = cache;
  process.env.VN_HOST_API = server.url.href;
  process.env.VN_HOST_REPO = "o/r";
  delete process.env.GH_TOKEN;
  delete process.env.GITHUB_TOKEN;
  release = 200;
  seen.length = 0;
  const m: HostManifest = {
    vueNative: VERSION,
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
  await expect(fetchHost({ variant: "no-audio", version: VERSION, platform: "windows", arch: "x64" })).rejects.toThrow(
    /no no-audio host for windows-x64 \(it has: standard\)/,
  );
});

test("a missing release says a private repository needs a token", async () => {
  publish();
  release = 404;
  await expect(fetchStandard()).rejects.toThrow(/has no v9\.9\.9-test release \(a private repository needs GH_TOKEN/);
});

test("a variant no release carries stands in with the published one that has every part it needs", () => {
  expect(standIn("no-devtools+no-audio")).toBe("no-devtools");
  expect(standIn("no-audio+no-webgpu")).toBe("standard");
  expect(standIn("no-devtools")).toBe("no-devtools");
  expect(standIn("no-devtools", ["standard"])).toBe(null);
});
