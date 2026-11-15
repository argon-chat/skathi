// `sk build` for Linux: the payload appended to a copy of the host ELF, found by its trailer the way
// crates/sk-payload `appended` finds it. Runs on any OS: the host is a fake ELF header.

import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildApp, summary } from "../cli/build-app";
import { loadAppConfig } from "../cli/config";
import {
  appendedPayload,
  appendedPayloadOf,
  elfMachine,
  elfMachineOf,
  stampElf,
  trailer,
  TRAILER_SIZE,
} from "../cli/elf";
import { cachedHost, isStamped, resolveHost } from "../cli/host";
import { currentPlatform, hostsFor, parsePlatform, repoHostExe } from "../cli/platform";
import { KIND, NAMES, PAGE, readSkpk, writeSkpk, type SkpkInput } from "../cli/skpk";

const dir = mkdtempSync(join(tmpdir(), "sk-elf-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));
let n = 0;
const fresh = (tag: string) => {
  const d = join(dir, `${tag}-${n++}`);
  mkdirSync(d, { recursive: true });
  return d;
};

function elfHeader(machine: number, cls = 2, data = 1): Uint8Array {
  const h = new Uint8Array(64);
  h.set([0x7f, 0x45, 0x4c, 0x46, cls, data, 1]);
  h[18] = machine & 0xff;
  h[19] = machine >> 8;
  return h;
}

function fakeElf(path: string, size = 5000): string {
  const bytes = new Uint8Array(size).map((_, i) => (i * 13 + 1) & 0xff);
  bytes.set(elfHeader(0x3e));
  writeFileSync(path, bytes);
  return path;
}

const enc = new TextEncoder();
const inputs = (tag: string): SkpkInput[] => [
  { name: NAMES.META, kind: KIND.META, data: enc.encode(JSON.stringify({ format: 1, name: tag })) },
  { name: NAMES.WINDOW, kind: KIND.WINDOW, data: enc.encode("{}") },
  { name: NAMES.SCRIPT, kind: KIND.SCRIPT, data: enc.encode(`console.log(${JSON.stringify(tag)})`) },
  { name: "assets/big.bin", kind: KIND.ASSET, data: new Uint8Array(70_000).map((_, i) => i * 7) },
];

describe("ELF sniffing", () => {
  test("64-bit little-endian x86_64 and aarch64; anything else is not a host", () => {
    expect(elfMachine(elfHeader(0x3e))).toBe("x86_64");
    expect(elfMachine(elfHeader(0xb7))).toBe("aarch64");
    expect(elfMachine(elfHeader(0x28))).toBe("machine 0x28");
    expect(elfMachine(elfHeader(0x3e, 1))).toBeNull();
    expect(elfMachine(elfHeader(0x3e, 2, 2))).toBeNull();
    expect(elfMachine(new Uint8Array([0x4d, 0x5a, 0, 0]))).toBeNull();
    expect(elfMachineOf(join(dir, "missing"))).toBeNull();
  });
});

describe("the trailer", () => {
  test("is u64 offset, u64 length, SKPKTAIL: the bytes the Rust reader's test pins", () => {
    expect(Buffer.from(trailer(4096, 1028)).toString("hex")).toBe(
      "0010000000000000" + "0404000000000000" + Buffer.from("SKPKTAIL").toString("hex"),
    );
  });

  test("names the payload only when it ends exactly at the trailer", () => {
    const file = new Uint8Array(4096 + 100 + TRAILER_SIZE);
    file.set(trailer(4096, 100), 4196);
    expect(appendedPayload(file, file.length)).toEqual({ offset: 4096, length: 100 });
    expect(appendedPayload(file, file.length + 1)).toBeNull();
    expect(appendedPayload(file.subarray(0, file.length - 1), file.length - 1)).toBeNull();
    expect(appendedPayload(new Uint8Array(8), 8)).toBeNull();
  });
});

describe("stamping", () => {
  test("host verbatim, zero pad to a page, the payload with its data page-aligned, the trailer; executable", () => {
    const d = fresh("stamp");
    const host = fakeElf(join(d, "sk-app"));
    const out = join(d, "Hello");
    const r = stampElf({ host, out, payload: (skew) => writeSkpk(inputs("hello"), { skew }) });
    const file = new Uint8Array(readFileSync(out));
    expect(r.payloadOffset).toBe(8192);
    expect(r.machine).toBe("x86_64");
    expect(r.bytes).toBe(file.length);
    expect(file.subarray(0, 5000)).toEqual(new Uint8Array(readFileSync(host)));
    expect(file.subarray(5000, 8192).every((b) => b === 0)).toBe(true);

    const at = appendedPayloadOf(out)!;
    expect(at).toEqual({ offset: 8192, length: r.payloadBytes });
    const payload = file.subarray(at.offset, at.offset + at.length);
    expect(Buffer.from(payload).equals(r.payload)).toBe(true);
    const skpk = readSkpk(payload);
    expect((at.offset + skpk.header.dataOff) % PAGE).toBe(0);
    expect(r.dataAligned).toBe(true);
    expect(new TextDecoder().decode(skpk.bytes(skpk.find(KIND.SCRIPT, NAMES.SCRIPT)!))).toBe('console.log("hello")');
    if (process.platform !== "win32") expect(statSync(out).mode & 0o111).toBe(0o111);

    expect(isStamped(out)).toBe(true);
    expect(isStamped(host)).toBe(false);
  });

  test("a host already a page long gets no pad; a stamped exe or a non-ELF is refused", () => {
    const d = fresh("refuse");
    const host = fakeElf(join(d, "sk-app"), PAGE);
    const r = stampElf({ host, out: join(d, "a"), payload: (skew) => writeSkpk(inputs("a"), { skew }) });
    expect(r.payloadOffset).toBe(PAGE);
    expect(() => stampElf({ host: r.out, out: join(d, "b"), payload: () => r.payload })).toThrow();
    writeFileSync(join(d, "pe.exe"), new Uint8Array([0x4d, 0x5a, 0, 0, 0, 0, 0, 0]));
    expect(() => stampElf({ host: join(d, "pe.exe"), out: join(d, "c"), payload: () => r.payload })).toThrow();
  });
});

describe("Linux hosts", () => {
  test("triple per CPU, no .exe, the cache under XDG_CACHE_HOME or ~/.cache", () => {
    expect(parsePlatform("linux")).toBe("linux");
    expect(hostsFor("linux", "x64").triple).toBe("x86_64-unknown-linux-gnu");
    expect(hostsFor("linux", "arm64").cargoTarget).toEqual(["--target", "aarch64-unknown-linux-gnu"]);
    expect(repoHostExe("R", hostsFor("linux", "x64"), "release")).toMatch(
      /x86_64-unknown-linux-gnu[\\/]release[\\/]sk-app$/,
    );
    const saved = process.env.XDG_CACHE_HOME;
    try {
      process.env.XDG_CACHE_HOME = join(dir, "xdg");
      expect(cachedHost("1.2.3", "standard", "linux")).toBe(
        join(dir, "xdg", "skathi", "hosts", "1.2.3", "standard", "sk-host"),
      );
      delete process.env.XDG_CACHE_HOME;
      expect(cachedHost("1.2.3", "standard", "linux")).toMatch(/\.cache[\\/]skathi[\\/]hosts[\\/]1\.2\.3/);
    } finally {
      if (saved === undefined) delete process.env.XDG_CACHE_HOME;
      else process.env.XDG_CACHE_HOME = saved;
    }
  });

  test("no icon warning: a Linux executable carries none", () => {
    const root = fresh("icon");
    writeFileSync(join(root, "main.ts"), "");
    writeFileSync(join(root, "sk.config.json"), JSON.stringify({ name: "x", platforms: ["linux"] }));
    const c = loadAppConfig(root, { platform: "linux" });
    expect(c.warnings.join("\n")).not.toMatch(/icon/);
    expect(c.platforms).toEqual(["linux"]);
  });

  describe.skipIf(currentPlatform() === "linux")("from another OS", () => {
    test("the host must be named and be an ELF", () => {
      const d = fresh("cross");
      expect(() => resolveHost({ variant: "standard", platform: "linux" })).toThrow();
      writeFileSync(join(d, "sk-app.exe"), new Uint8Array([0x4d, 0x5a, 0, 0, 0, 0, 0, 0]));
      expect(() => resolveHost({ variant: "standard", platform: "linux", host: join(d, "sk-app.exe") })).toThrow();
      const trusted = resolveHost({ variant: "standard", platform: "linux", host: fakeElf(join(d, "sk-app")) });
      expect(trusted.unverified).toBeDefined();
    });

    test("sk build --platform linux --host <fake>: the exe carries the build's payload, packs beside it", async () => {
      const root = fresh("e2e");
      mkdirSync(join(root, "src"));
      mkdirSync(join(root, "assets"));
      writeFileSync(join(root, "src", "main.ts"), "export const x = 1;\n");
      writeFileSync(join(root, "assets", "a.txt"), "hello");
      writeFileSync(join(root, "assets", "b.bin"), "bytes");
      writeFileSync(
        join(root, "sk.config.json"),
        JSON.stringify({
          name: "e2e",
          productName: "E2E Linux",
          version: "2.3.4",
          packs: [{ name: "text", match: ["**/*.txt"] }],
        }),
      );
      const host = fakeElf(join(fresh("e2e-host"), "sk-app"));
      const out = join(root, "dist");
      const r = await buildApp({ target: root, out, host, platform: "linux", log: () => {} });

      expect(r.platform).toBe("linux");
      expect(r.stamp).toBeNull();
      expect(r.bundle).toBeNull();
      expect(r.exe).toBe(join(out, "E2E Linux"));
      expect(r.elf!.dataAligned).toBe(true);
      expect(r.warnings.join("\n")).toMatch(/cannot be run/);
      expect(r.warnings.join("\n")).not.toMatch(/icon/);

      const exe = new Uint8Array(readFileSync(r.exe!));
      const at = appendedPayloadOf(r.exe!)!;
      expect(Buffer.from(exe.subarray(at.offset, at.offset + at.length)).equals(readFileSync(r.skpk))).toBe(true);
      const payload = readSkpk(exe.subarray(at.offset, at.offset + at.length));
      const meta = JSON.parse(new TextDecoder().decode(payload.bytes(payload.find(KIND.META, NAMES.META)!)));
      expect(meta).toMatchObject({ name: "e2e", version: "2.3.4", packs: [{ name: "text", file: "text.skpk" }] });
      expect(payload.find(KIND.ASSET, "assets/b.bin")).toBeDefined();
      expect(payload.find(KIND.ASSET, "assets/a.txt")).toBeUndefined();
      expect(
        readSkpk(new Uint8Array(readFileSync(join(out, "text.skpk")))).find(KIND.ASSET, "assets/a.txt"),
      ).toBeDefined();
      expect(summary(r)).toMatch(/~\/\.local\/share\/e2e\/storage/);
    }, 120_000);
  });
});
