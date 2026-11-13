// The stamper, on a SYSTEM exe (no cargo, no host, about a second): `C:\Windows\System32\whoami.exe` is a small console
// PE with a manifest, VERSIONINFO and MUI resource, the shape of the host for `UpdateResourceW`. Each test stamps a
// fresh copy and asks WINDOWS about it (loader, `FindResourceW`, `ExtractIconExW`, version API), not our own bytes.

import { afterAll, describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdtempSync, rmSync, statSync, writeFileSync, appendFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { encodePng, iconImagesFrom } from "../cli/icon";
import {
  fixedFileVersion,
  loadResourceCopy,
  patchFile,
  readFileRange,
  readPe,
  RT,
  shellIconCount,
  SUBSYSTEM,
  versionString,
} from "../cli/pe";
import { PAYLOAD_NAME, stamp } from "../cli/stamp";
import { buildVersionInfo } from "../cli/versioninfo";
import { KIND, NAMES, PAGE, readVnpk, writeVnpk, type VnpkInput } from "../cli/vnpk";

const WHOAMI = "C:\\Windows\\System32\\whoami.exe";
const isWindows = process.platform === "win32";
const dir = isWindows ? mkdtempSync(join(tmpdir(), "vn-stamp-")) : "";
afterAll(() => {
  if (dir) rmSync(dir, { recursive: true, force: true });
});

const enc = new TextEncoder();
function payloadInputs(tag: string): VnpkInput[] {
  return [
    { name: NAMES.META, kind: KIND.META, data: enc.encode(JSON.stringify({ format: 1, name: tag })) },
    { name: NAMES.WINDOW, kind: KIND.WINDOW, data: enc.encode("{}") },
    { name: NAMES.SCRIPT, kind: KIND.SCRIPT, data: enc.encode(`console.log(${JSON.stringify(tag)})`) },
    { name: "assets/hello.txt", kind: KIND.ASSET, data: enc.encode(`hello from ${tag}\n`) },
    { name: "assets/big.bin", kind: KIND.ASSET, data: new Uint8Array(300_000).map((_, i) => i * 7) },
  ];
}

function iconPng(): string {
  const size = 256;
  const data = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const o = (y * size + x) * 4;
      const inside = (x - 128) ** 2 + (y - 128) ** 2 < 110 ** 2;
      data[o] = 0x42;
      data[o + 1] = 0xb8;
      data[o + 2] = 0x83;
      data[o + 3] = inside ? 255 : 0;
    }
  }
  const path = join(dir, "icon.png");
  writeFileSync(path, encodePng({ width: size, height: size, data }));
  return path;
}

const versionInfo = () =>
  buildVersionInfo({
    fileVersion: [1, 2, 3, 0],
    strings: {
      FileDescription: "Stamped Test",
      ProductName: "Stamped Test",
      ProductVersion: "1.2.3-beta.1",
      FileVersion: "1.2.3.0",
      InternalName: "stamped",
      OriginalFilename: "Stamped.exe",
      CompanyName: "Example Co",
      LegalCopyright: "(c) 2026 Example Co",
    },
  });

const expectedUser = isWindows ? spawnSync(WHOAMI, { encoding: "utf8" }).stdout.trim() : "";

describe.skipIf(!isWindows)("stamping a real PE", () => {
  const host = join(dir, "host.exe");
  const out = join(dir, "Stamped.exe");
  let stamped: ReturnType<typeof stamp>;

  test("a stamped copy of whoami.exe still RUNS, as a GUI-subsystem image", () => {
    copyFileSync(WHOAMI, host);
    const inputs = payloadInputs("one");
    stamped = stamp({
      host,
      out,
      payload: (skew) => writeVnpk(inputs, { skew }),
      icons: iconImagesFrom(iconPng()),
      versionInfo: versionInfo(),
      subsystem: "gui",
    });
    expect(readPe(out).subsystem).toBe(SUBSYSTEM.GUI);
    expect(stamped.subsystem).toBe(SUBSYSTEM.GUI);
    // Inherited pipes still work for a GUI-subsystem child: what lets `vn run` read a packaged app's stderr.
    const run = spawnSync(out, { encoding: "utf8" });
    expect(run.status).toBe(0);
    expect(run.stdout.trim().toLowerCase()).toBe(expectedUser.toLowerCase());
  });

  test("the payload is where FindResourceW looks, byte for byte, data region page-aligned", () => {
    const mapped = loadResourceCopy(out, RT.RCDATA, PAYLOAD_NAME);
    expect(mapped).not.toBeNull();
    expect(mapped!.length).toBe(stamped.payloadBytes);
    const pk = readVnpk(mapped!);
    expect(pk.entries.map((e) => e.name)).toEqual([
      NAMES.META,
      NAMES.WINDOW,
      NAMES.SCRIPT,
      "assets/big.bin",
      "assets/hello.txt",
    ]);
    const hello = pk.find(KIND.ASSET, "assets/hello.txt")!;
    expect(new TextDecoder().decode(pk.bytes(hello))).toBe("hello from one\n");
    expect(stamped.dataAligned).toBe(true);
    expect((stamped.payloadRva + pk.header.dataOff) % PAGE).toBe(0);
    expect(stamped.passes).toBeLessThanOrEqual(2);
  });

  test("the shell sees the icon and the version API reads the strings back", () => {
    expect(shellIconCount(out)).toBe(1);
    expect(shellIconCount(WHOAMI)).toBe(0);
    expect(versionString(out, "ProductName")).toBe("Stamped Test");
    expect(versionString(out, "FileDescription")).toBe("Stamped Test");
    expect(versionString(out, "ProductVersion")).toBe("1.2.3-beta.1");
    expect(versionString(out, "CompanyName")).toBe("Example Co");
    expect(versionString(out, "OriginalFilename")).toBe("Stamped.exe");
    expect(fixedFileVersion(out)).toBe("1.2.3.0");
    const pe = readPe(out);
    expect(pe.resources.filter((r) => r.type === RT.VERSION)).toHaveLength(1);
    expect(pe.resources.filter((r) => r.type === RT.GROUP_ICON)).toHaveLength(1);
    expect(pe.resources.filter((r) => r.type === RT.ICON)).toHaveLength(5);
  });

  test("the host's manifest is left byte-identical", () => {
    const manifest = (p: string) => {
      const e = readPe(p).resources.find((r) => r.type === RT.MANIFEST)!;
      return readFileRange(p, e.fileOffset, e.size);
    };
    expect(Buffer.from(manifest(out)).equals(Buffer.from(manifest(WHOAMI)))).toBe(true);
  });

  test("re-stamping a stamped exe replaces the payload and keeps the identity", () => {
    const again = join(dir, "Restamped.exe");
    const inputs = payloadInputs("two");
    const r = stamp({ host: out, out: again, payload: (skew) => writeVnpk(inputs, { skew }) });
    const pk = readVnpk(loadResourceCopy(again, RT.RCDATA, PAYLOAD_NAME)!);
    expect(new TextDecoder().decode(pk.bytes(pk.find(KIND.ASSET, "assets/hello.txt")!))).toBe("hello from two\n");
    expect(r.dataAligned).toBe(true);
    expect(readPe(again).resources.filter((x) => x.name === PAYLOAD_NAME)).toHaveLength(1);
    expect(shellIconCount(again)).toBe(1);
    expect(versionString(again, "ProductName")).toBe("Stamped Test");
    expect(spawnSync(again, { encoding: "utf8" }).status).toBe(0);
  });

  test("an existing certificate table is stripped before the edit", () => {
    // whoami.exe is catalog-signed, so give a copy a fake certificate table at its tail, where signtool puts one.
    const signed = join(dir, "signed.exe");
    copyFileSync(WHOAMI, signed);
    const size = statSync(signed).size;
    appendFileSync(signed, new Uint8Array(512).fill(0xcc));
    const entry = new Uint8Array(8);
    new DataView(entry.buffer).setUint32(0, size, true);
    new DataView(entry.buffer).setUint32(4, 512, true);
    patchFile(signed, readPe(signed).securityEntryOffset, entry);
    expect(readPe(signed).security).toEqual({ offset: size, size: 512 });

    const outSigned = join(dir, "FromSigned.exe");
    const inputs = payloadInputs("signed");
    const r = stamp({ host: signed, out: outSigned, payload: (skew) => writeVnpk(inputs, { skew }) });
    expect(r.strippedSignature).toBe(true);
    expect(readPe(outSigned).security).toEqual({ offset: 0, size: 0 });
    expect(spawnSync(outSigned, { encoding: "utf8" }).status).toBe(0);
  });

  test("`subsystem: console` keeps a console app a console app", () => {
    const consoleOut = join(dir, "Console.exe");
    stamp({ host, out: consoleOut, payload: (skew) => writeVnpk(payloadInputs("c"), { skew }), subsystem: "console" });
    expect(readPe(consoleOut).subsystem).toBe(SUBSYSTEM.CONSOLE);
  });
});
