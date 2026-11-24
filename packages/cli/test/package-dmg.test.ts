// `sk package` for macOS: the .DS_Store, bplist and alias encoders read back by independent readers, the options,
// validation and the notarytool commands. hdiutil and notarytool themselves only run on a Mac.

import { afterAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadAppConfig } from "../cli/config";
import { encodePng } from "../cli/icon";
import { parsePackageConfig } from "../cli/package/config";
import {
  aliasRecord,
  bplist,
  dsStore,
  layoutRecords,
  mountPointFrom,
  notaryCredentials,
  notarySubmitArgs,
  parseNotaryResult,
  resolveDmg,
  validateDmg,
  type DsRecord,
} from "../cli/package/dmg";
import type { PackageCheckContext, Problem } from "../cli/package/types";

const dir = mkdtempSync(join(tmpdir(), "sk-dmg-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));
let seq = 0;

function app(config: Record<string, unknown>, files: Record<string, Uint8Array> = {}): string {
  const root = join(dir, `${seq++}`);
  mkdirSync(join(root, "src"), { recursive: true });
  writeFileSync(join(root, "src", "main.ts"), "");
  writeFileSync(
    join(root, "sk.config.json"),
    JSON.stringify({ name: "t", productName: "Notes", version: "1.2.3", id: "com.example.notes", ...config }),
  );
  for (const [k, v] of Object.entries(files)) writeFileSync(join(root, k), v);
  return root;
}

function checkContext(dmg: Record<string, unknown>, extra: Record<string, unknown> = {}, files = {}) {
  const a = loadAppConfig(app({ package: { macos: { dmg } }, ...extra }, files), { platform: "macos" });
  const parsed = parsePackageConfig(a);
  const ctx: PackageCheckContext = { appRoot: a.appRoot, app: a, config: parsed.config, platform: "macos" };
  return { ctx, problems: parsed.problems.filter((p) => p.key.startsWith("package.macos.dmg")) };
}

const codes = (ps: Problem[]) => ps.filter((p) => p.severity === "error").map((p) => `${p.code} ${p.key}`);
const png = encodePng({ width: 4, height: 4, data: new Uint8Array(64).fill(255) });

// ---- independent readers --------------------------------------------------------------------------------------

type Plist = string | number | boolean | Uint8Array | Plist[] | { [k: string]: Plist };

function readBplist(b: Uint8Array): Plist {
  expect(new TextDecoder().decode(b.subarray(0, 8))).toBe("bplist00");
  const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const t = b.length - 32;
  const offSize = b[t + 6]!;
  const refSize = b[t + 7]!;
  const count = Number(dv.getBigUint64(t + 8));
  const top = Number(dv.getBigUint64(t + 16));
  const table = Number(dv.getBigUint64(t + 24));
  const uint = (o: number, n: number) => {
    let v = 0;
    for (let i = 0; i < n; i++) v = v * 256 + b[o + i]!;
    return v;
  };
  const offset = (i: number) => {
    expect(i).toBeLessThan(count);
    return uint(table + i * offSize, offSize);
  };
  const read = (i: number): Plist => {
    let o = offset(i);
    const m = b[o]!;
    const hi = m >> 4;
    let n = m & 0xf;
    if (m === 0x08 || m === 0x09) return m === 0x09;
    if (hi === 0x1) return n === 3 ? Number(dv.getBigInt64(o + 1)) : uint(o + 1, 1 << n);
    if (m === 0x23) return dv.getFloat64(o + 1);
    o++;
    if (n === 0xf) {
      const s = b[o]! & 0xf;
      n = uint(o + 1, 1 << s);
      o += 1 + (1 << s);
    }
    if (hi === 0x4) return b.slice(o, o + n);
    if (hi === 0x5) return new TextDecoder().decode(b.subarray(o, o + n));
    if (hi === 0x6) return String.fromCharCode(...Array.from({ length: n }, (_, k) => dv.getUint16(o + 2 * k)));
    if (hi === 0xd) {
      const d: { [k: string]: Plist } = {};
      for (let k = 0; k < n; k++) {
        d[read(uint(o + k * refSize, refSize)) as string] = read(uint(o + (n + k) * refSize, refSize));
      }
      return d;
    }
    throw new Error(`marker 0x${m.toString(16)}`);
  };
  return read(top);
}

interface Store {
  records: { name: string; code: string; type: string; value: Uint8Array | number }[];
  blocks: [number, number][];
  free: [number, number][];
}

function readStore(f: Uint8Array): Store {
  const dv = new DataView(f.buffer, f.byteOffset, f.byteLength);
  const at = (addr: number) => 4 + addr;
  expect(dv.getUint32(0)).toBe(1);
  expect(new TextDecoder().decode(f.subarray(4, 8))).toBe("Bud1");
  const rootOff = dv.getUint32(8);
  expect(dv.getUint32(16)).toBe(rootOff);
  let o = at(rootOff);
  const count = dv.getUint32(o);
  o += 8;
  const addrs = Array.from({ length: count }, (_, i) => dv.getUint32(o + i * 4));
  o += 4 * (Math.ceil(count / 256) * 256);
  const toc = new Map<string, number>();
  const tocCount = dv.getUint32(o);
  o += 4;
  for (let i = 0; i < tocCount; i++) {
    const len = f[o]!;
    toc.set(new TextDecoder().decode(f.subarray(o + 1, o + 1 + len)), dv.getUint32(o + 1 + len));
    o += 1 + len + 4;
  }
  const free: [number, number][] = [];
  for (let w = 0; w < 32; w++) {
    const n = dv.getUint32(o);
    o += 4;
    for (let i = 0; i < n; i++, o += 4) free.push([dv.getUint32(o), 2 ** w]);
  }
  const block = (id: number) => at(addrs[id]! & ~0x1f);
  const dsdb = block(toc.get("DSDB")!);
  const [rootNode, levels, nRecords, nodes, page] = [0, 4, 8, 12, 16].map((k) => dv.getUint32(dsdb + k));
  expect([levels, nodes, page]).toEqual([0, 1, 0x1000]);
  let p = block(rootNode!);
  expect(dv.getUint32(p)).toBe(0);
  const n = dv.getUint32(p + 4);
  expect(n).toBe(nRecords!);
  p += 8;
  const records: Store["records"] = [];
  const four = (q: number) => new TextDecoder().decode(f.subarray(q, q + 4));
  for (let i = 0; i < n; i++) {
    const len = dv.getUint32(p);
    const name = String.fromCharCode(...Array.from({ length: len }, (_, k) => dv.getUint16(p + 4 + 2 * k)));
    p += 4 + 2 * len;
    const code = four(p);
    const type = four(p + 4);
    p += 8;
    if (type === "blob") {
      const l = dv.getUint32(p);
      records.push({ name, code, type, value: f.slice(p + 4, p + 4 + l) });
      p += 4 + l;
    } else if (type === "long") {
      records.push({ name, code, type, value: dv.getUint32(p) });
      p += 4;
    } else if (type === "type") {
      records.push({ name, code, type, value: f.slice(p, p + 4) });
      p += 4;
    } else throw new Error(`type ${type}`);
  }
  const blocks: [number, number][] = [[0, 32], ...addrs.map((a): [number, number] => [a & ~0x1f, 2 ** (a & 0x1f)])];
  return { records, blocks, free };
}

// ---- tests ----------------------------------------------------------------------------------------------------

describe(".DS_Store", () => {
  const dmg = resolveDmg({}, "Notes", "1.2.3");
  const records = layoutRecords(dmg, "Notes.app", { alias: new Uint8Array([1, 2, 3]) });

  test("reads back: records sorted case-insensitively, '.' first, every value intact", () => {
    const shuffled: DsRecord[] = [...records].reverse();
    const store = readStore(dsStore(shuffled));
    expect(store.records.map((r) => `${r.name}/${r.code}`)).toEqual([
      "./bwsp",
      "./icvp",
      "./vSrn",
      "./vstl",
      "Applications/Iloc",
      "Notes.app/Iloc",
    ]);
    const iloc = store.records.find((r) => r.name === "Notes.app")!.value as Uint8Array;
    const dv = new DataView(iloc.buffer);
    expect([dv.getUint32(0), dv.getUint32(4)]).toEqual([dmg.appPosition.x, dmg.appPosition.y]);
    expect(Array.from(iloc.subarray(8))).toEqual([255, 255, 255, 255, 255, 255, 0, 0]);
    expect(store.records.find((r) => r.code === "vSrn")!.value).toBe(1);
  });

  test("the buddy blocks and free lists tile the 2 GiB address space without overlap", () => {
    const store = readStore(dsStore(records));
    const all = [...store.blocks, ...store.free].sort((a, b) => a[0] - b[0]);
    let next = 0;
    for (const [off, size] of all) {
      expect(off % size).toBe(0);
      expect(off).toBe(next);
      next = off + size;
    }
    expect(next).toBe(2 ** 31);
  });

  test("the window and icon view plists", () => {
    const store = readStore(dsStore(layoutRecords({ ...dmg, backgroundColor: "#336699" }, "Notes.app", null)));
    const bwsp = readBplist(store.records.find((r) => r.code === "bwsp")!.value as Uint8Array) as Record<string, Plist>;
    expect(bwsp.WindowBounds).toBe("{{200, 120}, {660, 400}}");
    expect(bwsp.ShowToolbar).toBe(false);
    const icvp = readBplist(store.records.find((r) => r.code === "icvp")!.value as Uint8Array) as Record<string, Plist>;
    expect(icvp.backgroundType).toBe(1);
    expect(icvp.backgroundColorRed).toBeCloseTo(0x33 / 255);
    expect(icvp.iconSize).toBe(128);
    expect(icvp.arrangeBy).toBe("none");
    expect(icvp.backgroundImageAlias).toBeUndefined();
  });

  test("a background picture: type 2 with its alias", () => {
    const store = readStore(dsStore(records));
    const icvp = readBplist(store.records.find((r) => r.code === "icvp")!.value as Uint8Array) as Record<string, Plist>;
    expect(icvp.backgroundType).toBe(2);
    expect(Array.from(icvp.backgroundImageAlias as Uint8Array)).toEqual([1, 2, 3]);
  });
});

describe("bplist", () => {
  test("long, non-ASCII and large values round-trip", () => {
    const value = {
      long: "x".repeat(300),
      unicode: "Заметки ✓",
      int: 70000,
      neg: -5,
      real: { real: 0.25 },
      data: new Uint8Array(20).fill(7),
      nested: { yes: true, no: false },
    };
    const many = Object.fromEntries(Array.from({ length: 140 }, (_, i) => [`k${i}`, i]));
    expect(readBplist(bplist({ ...value, ...many }))).toEqual({
      ...value,
      real: 0.25,
      ...many,
    } as unknown as Plist);
  });
});

describe("alias", () => {
  test("the record's size, version, ids and paths", () => {
    const a = aliasRecord({
      volumeName: "Notes",
      volumeCreated: new Date(Date.UTC(2026, 0, 1)),
      path: "/.background/background.png",
      cnid: 42,
      parentCnid: 17,
      created: new Date(Date.UTC(2026, 0, 2)),
    });
    const dv = new DataView(a.buffer);
    expect(dv.getUint16(4)).toBe(a.length);
    expect(dv.getUint16(6)).toBe(2);
    expect(a[10]).toBe(5);
    expect(new TextDecoder().decode(a.subarray(11, 16))).toBe("Notes");
    expect(dv.getUint32(8 + 30)).toBe(Date.UTC(2026, 0, 1) / 1000 + 2082844800);
    expect(dv.getUint32(8 + 38)).toBe(17);
    expect(dv.getUint32(8 + 106)).toBe(42);
    const tags = new Map<number, string>();
    for (let o = 150; ;) {
      const id = dv.getInt16(o);
      const len = dv.getUint16(o + 2);
      if (id === -1) break;
      tags.set(id, new TextDecoder().decode(a.subarray(o + 4, o + 4 + len)));
      o += 4 + len + (len % 2);
    }
    expect(tags.get(18)).toBe("/.background/background.png");
    expect(tags.get(19)).toBe("/Volumes/Notes");
    expect(tags.get(2)).toBe("Notes:.background:background.png");
  });
});

describe("options", () => {
  test("defaults from the identity, a partial window keeps the other defaults, paths from the app root", () => {
    const { ctx, problems } = checkContext({ window: { width: 800 }, background: "bg.png", format: "ULMO" });
    expect(problems).toEqual([]);
    const d = resolveDmg(ctx.config.macos.dmg, "x", "0");
    expect(d.volumeName).toBe("Notes");
    expect(d.fileName).toBe("Notes-1.2.3-{arch}");
    expect(d.window).toEqual({ x: 200, y: 120, width: 800, height: 400 });
    expect(d.background).toBe(join(ctx.appRoot, "bg.png"));
    expect(d.format).toBe("ULMO");
  });

  test("bad values and unknown keys are problems", () => {
    const { problems } = checkContext({
      format: "UDRW",
      iconSize: 2,
      fileName: "a/b",
      backgroundColor: "red",
      notarize: { keychainProfile: "p", apiKey: "env:K" },
      positions: {},
    });
    expect(codes(problems).sort()).toEqual([
      "SK8001 package.macos.dmg.positions",
      "SK8202 package.macos.dmg.backgroundColor",
      "SK8202 package.macos.dmg.fileName",
      "SK8202 package.macos.dmg.format",
      "SK8202 package.macos.dmg.iconSize",
      "SK8202 package.macos.dmg.notarize",
    ]);
  });
});

describe("validate", () => {
  const signed = { macos: { sign: "Developer ID Application: Example (TEAM123456)" } };

  test("other hosts are told to use a Mac, without failing sk check there", () => {
    const { ctx } = checkContext({});
    const elsewhere = validateDmg(ctx, "win32");
    expect(elsewhere.map((p) => `${p.severity} ${p.code}`)).toEqual(["warning SK8201"]);
    expect(validateDmg(ctx, "darwin")).toEqual([]);
  });

  test("a literal API key is refused; env references and literal ids pass", () => {
    const literal = checkContext(
      { notarize: { apiKey: "-----BEGIN PRIVATE KEY-----\nMIG…", apiKeyId: "ABC123", apiIssuer: "uuid" } },
      signed,
    );
    expect(codes(validateDmg(literal.ctx, "darwin"))).toEqual(["SK8204 package.macos.dmg.notarize.apiKey"]);
    const path = checkContext({ notarize: { apiKey: "keys/AuthKey.p8", apiKeyId: "A", apiIssuer: "B" } }, signed);
    expect(codes(validateDmg(path.ctx, "darwin"))).toEqual(["SK8204 package.macos.dmg.notarize.apiKey"]);
    const env = checkContext(
      { notarize: { apiKey: "env:ASC_KEY", apiKeyId: "ABC123", apiIssuer: "env:ASC_ISSUER" } },
      signed,
    );
    expect(codes(validateDmg(env.ctx, "darwin"))).toEqual([]);
  });

  test("notarisation needs a Developer ID identity on the app", () => {
    for (const macos of [{}, { sign: "-" }, { sign: false }]) {
      const { ctx } = checkContext({ notarize: { keychainProfile: "notary" } }, { macos });
      expect(codes(validateDmg(ctx, "darwin"))).toEqual(["SK8205 package.macos.dmg.notarize"]);
    }
  });

  test("the background must exist and be a PNG or TIFF", () => {
    expect(codes(validateDmg(checkContext({ background: "none.png" }).ctx, "darwin"))).toEqual([
      "SK8203 package.macos.dmg.background",
    ]);
    const notPng = checkContext({ background: "bg.png" }, {}, { "bg.png": new TextEncoder().encode("GIF89a…") });
    expect(codes(validateDmg(notPng.ctx, "darwin"))).toEqual(["SK8203 package.macos.dmg.background"]);
    const ok = checkContext({ background: "bg.png" }, {}, { "bg.png": png });
    expect(codes(validateDmg(ok.ctx, "darwin"))).toEqual([]);
  });
});

describe("notarytool", () => {
  const api = { apiKey: "env:ASC_KEY", apiKeyId: "env:ASC_KEY_ID", apiIssuer: "issuer-uuid" };

  test("an API key's PEM text goes to a 0600 file, never onto the command line", () => {
    const keyFile = join(dir, "AuthKey.p8");
    const pem = "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----";
    const c = notaryCredentials(api, { ASC_KEY: pem, ASC_KEY_ID: "KID" }, keyFile);
    expect(readFileSync(keyFile, "utf8")).toBe(`${pem}\n`);
    if (process.platform !== "win32") expect(statSync(keyFile).mode & 0o777).toBe(0o600);
    const args = notarySubmitArgs("/o/N.dmg", c);
    expect(args.join(" ")).not.toContain("BEGIN");
    expect(args).toEqual([
      "notarytool",
      "submit",
      "/o/N.dmg",
      "--key",
      keyFile,
      "--key-id",
      "KID",
      "--issuer",
      "issuer-uuid",
      "--wait",
      "--output-format",
      "json",
    ]);
  });

  test("a key path in the variable is passed as is; a missing variable or file is an error", () => {
    const p8 = join(dir, "real.p8");
    writeFileSync(p8, "k");
    const keyFile = join(dir, "unused.p8");
    expect(notaryCredentials(api, { ASC_KEY: p8, ASC_KEY_ID: "K" }, keyFile)).toEqual({
      keyPath: p8,
      keyId: "K",
      issuer: "issuer-uuid",
    });
    expect(existsSync(keyFile)).toBe(false);
    expect(() => notaryCredentials(api, { ASC_KEY_ID: "K" }, keyFile)).toThrow(/ASC_KEY\b/);
    expect(() => notaryCredentials(api, { ASC_KEY: p8 }, keyFile)).toThrow(/ASC_KEY_ID/);
    expect(() => notaryCredentials(api, { ASC_KEY: join(dir, "gone.p8"), ASC_KEY_ID: "K" }, keyFile)).toThrow();
    expect(() => notaryCredentials({ ...api, apiKey: "-----BEGIN PRIVATE KEY-----" }, {}, keyFile)).toThrow();
  });

  test("a keychain profile, with and without its keychain", () => {
    const c = notaryCredentials({ keychainProfile: "notary", keychain: null }, {}, "");
    expect(notarySubmitArgs("a.zip", c).slice(3, 5)).toEqual(["--keychain-profile", "notary"]);
    const k = notaryCredentials({ keychainProfile: "notary", keychain: "/k/ci.keychain-db" }, {}, "");
    expect(notarySubmitArgs("a.zip", k).slice(3, 7)).toEqual([
      "--keychain-profile",
      "notary",
      "--keychain",
      "/k/ci.keychain-db",
    ]);
  });

  test("the submission result: accepted, rejected, and output that is not JSON", () => {
    expect(parseNotaryResult('{"id":"1","status":"Invalid","message":"Processing complete"}')).toEqual({
      id: "1",
      status: "Invalid",
      message: "Processing complete",
    });
    expect(() => parseNotaryResult("Error: HTTP 401")).toThrow();
    expect(() => parseNotaryResult('{"message":"x"}')).toThrow();
  });
});

describe("hdiutil", () => {
  test("the mount point from attach -plist, with XML escapes", () => {
    const plist =
      "<plist><dict><key>system-entities</key><array><dict><key>content-hint</key><string>GUID_partition_scheme</string></dict>" +
      "<dict><key>mount-point</key><string>/private/tmp/dmg.Ab12 &amp; Co</string></dict></array></dict></plist>";
    expect(mountPointFrom(plist)).toBe("/private/tmp/dmg.Ab12 & Co");
    expect(() => mountPointFrom("<plist/>")).toThrow();
  });
});
