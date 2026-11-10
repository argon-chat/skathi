// `vn build` for macOS: the `.app` layout, Info.plist, the icns encoder, the
// `macos` config block and host resolution per platform (cli/macos-bundle.ts,
// cli/platform.ts, docs/PACKAGING.md §10).
//
//   bun test packages/vue-native/test/macos-bundle.test.ts
//
// Everything here runs on any OS: the bundle is files, the plist is text, the
// icns is bytes, and the host is a fake Mach-O header that is never run.
// Whether a real Mac launches the result is docs/MACOS.md §5's question.

import { afterAll, describe, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildApp } from "../cli/build-app";
import { ConfigError, loadAppConfig, MACOS_MINIMUM } from "../cli/config";
import { cachedHost, crossHostInfo, HostError, isStamped, resolveHost } from "../cli/host";
import { decodePng, encodePng, type Rgba } from "../cli/icon";
import {
  AD_HOC,
  bundlePayloadPath,
  codesignArgs,
  encodeIcns,
  entitlements,
  ICNS_TYPES,
  icnsFrom,
  icnsFromPng,
  infoPlist,
  machOArchs,
  parseIcns,
  plistXml,
  signPlan,
  writeAppBundle,
  type PlistValue,
} from "../cli/macos-bundle";
import { currentPlatform, hostsFor, parsePlatform, repoHostExe } from "../cli/platform";
import { KIND, NAMES, readVnpk } from "../cli/vnpk";

const dir = mkdtempSync(join(tmpdir(), "vn-macos-"));
afterAll(() => rmSync(dir, { recursive: true, force: true }));
let seq = 0;
const fresh = (name: string) => {
  const d = join(dir, `${seq++}-${name}`);
  mkdirSync(d, { recursive: true });
  return d;
};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/** Just enough of an XML plist reader to read back what plistXml writes. */
function readPlist(xml: string): PlistValue {
  expect(xml.startsWith('<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE plist')).toBe(true);
  expect(xml).toContain('<plist version="1.0">');
  const body = xml.replace(/<\?xml[^>]*\?>|<!DOCTYPE[^>]*>|<plist version="1.0">|<\/plist>/g, "");
  const tokens = [...body.matchAll(/<(\/?)([a-z]+)(\/?)>|([^<]+)/g)];
  let i = 0;
  const unescape = (s: string) => s.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
  const skipWs = () => {
    while (i < tokens.length && tokens[i]![4] !== undefined && !tokens[i]![4]!.trim()) i++;
  };
  const text = (tag: string): string => {
    let s = "";
    while (tokens[i]![4] !== undefined) s += tokens[i++]![4];
    expect(tokens[i]![1] + tokens[i]![2]).toBe(`/${tag}`);
    i++;
    return unescape(s);
  };
  const value = (): PlistValue => {
    skipWs();
    const [, close, tag, empty] = tokens[i++]!;
    expect(close).toBe("");
    if (tag === "true" || tag === "false") return tag === "true";
    if (tag === "string") return empty ? "" : text("string");
    if (tag === "integer" || tag === "real") return Number(text(tag));
    if (tag === "array") {
      const out: PlistValue[] = [];
      if (empty) return out;
      for (;;) {
        skipWs();
        if (tokens[i]![1] === "/") return i++, out;
        out.push(value());
      }
    }
    if (tag === "dict") {
      const out: Record<string, PlistValue> = {};
      if (empty) return out;
      for (;;) {
        skipWs();
        if (tokens[i]![1] === "/") return i++, out;
        expect(tokens[i++]![2]).toBe("key");
        const k = text("key");
        out[k] = value();
      }
    }
    throw new Error(`unexpected <${tag}>`);
  };
  return value();
}

function solid(size: number, rgba: [number, number, number, number] = [200, 40, 90, 255]): Rgba {
  const data = new Uint8Array(size * size * 4);
  for (let i = 0; i < size * size; i++) data.set(rgba, i * 4);
  return { width: size, height: size, data };
}

/** A Mach-O header the sniffer accepts; never executed. */
function fakeMachO(path: string, cpu = 0x0100000c): string {
  const b = new Uint8Array(4096);
  const dv = new DataView(b.buffer);
  dv.setUint32(0, 0xfeedfacf, true);
  dv.setUint32(4, cpu, true);
  writeFileSync(path, b);
  return path;
}

const BASE_INFO = {
  bundleId: "gl.argon.hello",
  name: "Hello",
  executable: "Hello",
  version: "1.2.3",
  iconFile: "AppIcon",
  minimumSystemVersion: "13.0",
};

// ---------------------------------------------------------------------------
// plist
// ---------------------------------------------------------------------------

describe("Info.plist", () => {
  test("the required keys, and nothing optional when nothing optional is configured", () => {
    const d = readPlist(plistXml(infoPlist(BASE_INFO))) as Record<string, PlistValue>;
    expect(d).toEqual({
      CFBundleDevelopmentRegion: "en",
      CFBundleDisplayName: "Hello",
      CFBundleExecutable: "Hello",
      CFBundleIconFile: "AppIcon",
      CFBundleIdentifier: "gl.argon.hello",
      CFBundleInfoDictionaryVersion: "6.0",
      CFBundleName: "Hello",
      CFBundlePackageType: "APPL",
      CFBundleShortVersionString: "1.2.3",
      CFBundleSignature: "????",
      CFBundleVersion: "1.2.3",
      LSMinimumSystemVersion: "13.0",
      NSHighResolutionCapable: true,
    });
  });

  test("usage descriptions, URL schemes, category and copyright appear only when set", () => {
    const d = infoPlist({
      ...BASE_INFO,
      iconFile: null,
      category: "public.app-category.social-networking",
      copyright: "© 2026 Argon",
      microphoneUsageDescription: "Voice rooms use the microphone.",
      cameraUsageDescription: "Video calls use the camera.",
      urlSchemes: ["argon", "argon-dev"],
    });
    expect(d.CFBundleIconFile).toBeUndefined();
    expect(d.LSApplicationCategoryType).toBe("public.app-category.social-networking");
    expect(d.NSHumanReadableCopyright).toBe("© 2026 Argon");
    expect(d.NSMicrophoneUsageDescription).toBe("Voice rooms use the microphone.");
    expect(d.NSCameraUsageDescription).toBe("Video calls use the camera.");
    expect(d.NSCameraUseContinuityCameraDeviceType).toBe(true);
    expect(d.CFBundleURLTypes).toEqual([{ CFBundleURLName: "gl.argon.hello", CFBundleURLSchemes: ["argon", "argon-dev"] }]);
    expect(readPlist(plistXml(d))).toEqual(d);
  });

  test("text is escaped, and what XML cannot carry is an error rather than a broken plist", () => {
    const name = `Tom & Jerry's <"Show"> ]]> 東京 😀`;
    const xml = plistXml(infoPlist({ ...BASE_INFO, name }));
    expect(xml).toContain("<string>Tom &amp; Jerry's &lt;\"Show\"&gt; ]]&gt; 東京 😀</string>");
    expect((readPlist(xml) as Record<string, PlistValue>).CFBundleName).toBe(name);
    expect(() => plistXml({ k: "bell \u0007" })).toThrow(/U\+0007/);
    expect(() => plistXml({ k: "half \uD800 pair" })).toThrow(/U\+d800/);
    expect(() => plistXml({ "bad\u0001key": "v" })).toThrow(/U\+0001/);
  });

  test("numbers, booleans and empty containers keep their plist types", () => {
    const v = { i: 3, r: 1.5, t: true, f: false, a: [], d: {}, nested: [{ x: ["y"] }] };
    const xml = plistXml(v);
    expect(xml).toContain("<integer>3</integer>");
    expect(xml).toContain("<real>1.5</real>");
    expect(xml).toContain("<array/>");
    expect(xml).toContain("<dict/>");
    expect(readPlist(xml)).toEqual(v);
    expect(() => plistXml({ n: Number.NaN })).toThrow(/not a plist number/);
  });

  test("entitlements: the JIT always, the microphone and camera only with a usage description", () => {
    expect(entitlements({})).toEqual({ "com.apple.security.cs.allow-jit": true });
    expect(entitlements({ microphoneUsageDescription: "m", cameraUsageDescription: "c" })).toEqual({
      "com.apple.security.cs.allow-jit": true,
      "com.apple.security.device.audio-input": true,
      "com.apple.security.device.camera": true,
    });
  });
});

// ---------------------------------------------------------------------------
// icns
// ---------------------------------------------------------------------------

describe("icns", () => {
  test("container: magic, big-endian total length, entries with OSType + length", () => {
    const a = new Uint8Array([1, 2, 3]);
    const b = new Uint8Array(300).fill(7);
    const bytes = encodeIcns([
      { type: "ic07", data: a },
      { type: "ic10", data: b },
    ]);
    expect(String.fromCharCode(...bytes.subarray(0, 4))).toBe("icns");
    const dv = new DataView(bytes.buffer);
    expect(dv.getUint32(4)).toBe(bytes.length);
    expect(bytes.length).toBe(8 + 8 + 3 + 8 + 300);
    expect(String.fromCharCode(...bytes.subarray(8, 12))).toBe("ic07");
    expect(dv.getUint32(12)).toBe(11);
    const back = parseIcns(bytes);
    expect(back.map((e) => e.type)).toEqual(["ic07", "ic10"]);
    expect([...back[0]!.data]).toEqual([1, 2, 3]);
    expect(back[1]!.data.length).toBe(300);
  });

  test("a malformed icns is refused", () => {
    const good = encodeIcns([{ type: "ic07", data: new Uint8Array(4) }]);
    expect(() => parseIcns(new Uint8Array([1, 2, 3, 4, 0, 0, 0, 8]))).toThrow(/magic/);
    const short = good.slice(0, good.length - 1);
    expect(() => parseIcns(short)).toThrow(/header says/);
    const lying = good.slice();
    new DataView(lying.buffer).setUint32(12, 999);
    expect(() => parseIcns(lying)).toThrow(/claims 999/);
    expect(() => encodeIcns([{ type: "toolong", data: new Uint8Array() }])).toThrow(/OSType/);
  });

  test("a 1024 px PNG becomes every PNG type, each a PNG of its size, the 1024 one verbatim", () => {
    const png = encodePng(solid(1024));
    const icns = icnsFromPng(png);
    expect(icns.sizes).toEqual([16, 32, 64, 128, 256, 512, 1024]);
    const entries = parseIcns(icns.bytes);
    expect(entries.map((e) => e.type)).toEqual(ICNS_TYPES.map(([t]) => t));
    for (const e of entries) {
      const size = ICNS_TYPES.find(([t]) => t === e.type)![1];
      const img = decodePng(e.data);
      expect([img.width, img.height]).toEqual([size, size]);
      expect([...img.data.subarray(0, 4)]).toEqual([200, 40, 90, 255]);
    }
    expect(Buffer.from(entries.find((e) => e.type === "ic10")!.data).equals(Buffer.from(png))).toBe(true);
    // @2x types reuse the 1x image of the same pixel size.
    const at = (t: string) => entries.find((e) => e.type === t)!.data;
    expect(at("ic11")).toEqual(at("icp5"));
    expect(at("ic13")).toEqual(at("ic08"));
  });

  test("never upscales: a 256 px PNG fills the types up to 256 and stops", () => {
    const icns = icnsFromPng(encodePng(solid(256)));
    expect(icns.sizes).toEqual([16, 32, 64, 128, 256]);
    expect(parseIcns(icns.bytes).map((e) => e.type)).toEqual(["icp4", "ic11", "icp5", "ic12", "ic07", "ic13", "ic08"]);
  });

  test("non-square and tiny PNGs are errors", () => {
    expect(() => icnsFromPng(encodePng({ width: 32, height: 16, data: new Uint8Array(32 * 16 * 4) }))).toThrow(/square/);
    expect(() => icnsFromPng(encodePng(solid(8)))).toThrow(/smaller than/);
  });

  test("icnsFrom: an .icns verbatim, an .ico's largest PNG converted, a BMP-only .ico refused", () => {
    const d = fresh("icons");
    const icns = encodeIcns([{ type: "ic08", data: encodePng(solid(256)) }]);
    writeFileSync(join(d, "a.icns"), icns);
    const verbatim = icnsFrom(join(d, "a.icns"));
    expect(verbatim.verbatim).toBe(true);
    expect(verbatim.sizes).toEqual([256]);
    expect(Buffer.from(verbatim.bytes).equals(Buffer.from(icns))).toBe(true);

    const ico = (images: { size: number; data: Uint8Array }[]) => {
      const head = new Uint8Array(6 + images.length * 16);
      const dv = new DataView(head.buffer);
      dv.setUint16(2, 1, true);
      dv.setUint16(4, images.length, true);
      let off = head.length;
      images.forEach((img, i) => {
        const e = 6 + i * 16;
        head[e] = img.size >= 256 ? 0 : img.size;
        head[e + 1] = img.size >= 256 ? 0 : img.size;
        dv.setUint16(e + 4, 1, true);
        dv.setUint16(e + 6, 32, true);
        dv.setUint32(e + 8, img.data.length, true);
        dv.setUint32(e + 12, off, true);
        off += img.data.length;
      });
      return Buffer.concat([Buffer.from(head), ...images.map((i) => Buffer.from(i.data))]);
    };
    writeFileSync(join(d, "a.ico"), ico([{ size: 32, data: encodePng(solid(32)) }, { size: 256, data: encodePng(solid(256)) }]));
    expect(icnsFrom(join(d, "a.ico")).sizes).toEqual([16, 32, 64, 128, 256]);
    writeFileSync(join(d, "bmp.ico"), ico([{ size: 32, data: new Uint8Array(40) }]));
    expect(() => icnsFrom(join(d, "bmp.ico"))).toThrow(/only BMP images/);
    writeFileSync(join(d, "x.jpg"), new Uint8Array([0xff, 0xd8, 0xff]));
    expect(() => icnsFrom(join(d, "x.jpg"))).toThrow(/\.icns, a square \.png/);
  });
});

// ---------------------------------------------------------------------------
// Mach-O and the bundle
// ---------------------------------------------------------------------------

describe("Mach-O sniffing", () => {
  test("thin arm64 and x86_64, universal, and things that are not Mach-O", () => {
    const d = fresh("macho");
    expect(machOArchs(fakeMachO(join(d, "arm")))).toEqual(["arm64"]);
    expect(machOArchs(fakeMachO(join(d, "x64"), 0x01000007))).toEqual(["x86_64"]);
    const fat = new Uint8Array(4096);
    const dv = new DataView(fat.buffer);
    dv.setUint32(0, 0xcafebabe);
    dv.setUint32(4, 2);
    dv.setUint32(8, 0x01000007);
    dv.setUint32(28, 0x0100000c);
    writeFileSync(join(d, "fat"), fat);
    expect(machOArchs(join(d, "fat"))).toEqual(["x86_64", "arm64"]);
    // A Java class file shares the magic; its "count" is a version number.
    dv.setUint32(4, 0x34);
    writeFileSync(join(d, "class"), fat);
    expect(machOArchs(join(d, "class"))).toBeNull();
    writeFileSync(join(d, "pe.exe"), new Uint8Array([0x4d, 0x5a, 0x90, 0, 3, 0, 0, 0, 4, 0]));
    expect(machOArchs(join(d, "pe.exe"))).toBeNull();
    expect(machOArchs(join(d, "missing"))).toBeNull();
  });

  test("the payload path is payload.rs's rule: only an executable in *.app/Contents/MacOS has one", () => {
    expect(bundlePayloadPath(join("/Applications", "Argon.app", "Contents", "MacOS", "Argon"))).toBe(
      join("/Applications", "Argon.app", "Contents", "Resources", "app.vnpk"),
    );
    expect(bundlePayloadPath(join("/x", "Argon.APP", "Contents", "MacOS", "a"))).not.toBeNull();
    expect(bundlePayloadPath(join("/tmp", "Contents", "MacOS", "vn-app"))).toBeNull();
    expect(bundlePayloadPath(join("/Applications", "Argon.app", "Contents", "Resources", "vn-app"))).toBeNull();
  });
});

describe("the .app", () => {
  const write = (out: string, host: string, payload = new Uint8Array([1, 2, 3, 4])) =>
    writeAppBundle({
      host,
      out,
      executable: "Hello",
      payload,
      icns: encodeIcns([{ type: "ic07", data: new Uint8Array(5) }]),
      infoPlist: plistXml(infoPlist({ ...BASE_INFO })),
    });

  test("layout: the host verbatim in MacOS (0755), payload and icon in Resources, Info.plist, PkgInfo", () => {
    const d = fresh("bundle");
    const host = fakeMachO(join(d, "vn-app"));
    const r = write(join(d, "Hello.app"), host);
    const c = join(d, "Hello.app", "Contents");
    expect(r.executable).toBe(join(c, "MacOS", "Hello"));
    expect(readdirSync(c).sort()).toEqual(["Info.plist", "MacOS", "PkgInfo", "Resources"]);
    expect(readdirSync(join(c, "MacOS"))).toEqual(["Hello"]);
    expect(readdirSync(join(c, "Resources")).sort()).toEqual(["AppIcon.icns", "app.vnpk"]);
    expect(readFileSync(join(c, "MacOS", "Hello")).equals(readFileSync(host))).toBe(true);
    expect([...readFileSync(join(c, "Resources", "app.vnpk"))]).toEqual([1, 2, 3, 4]);
    expect(readFileSync(join(c, "PkgInfo"), "utf8")).toBe("APPL????");
    const info = readPlist(readFileSync(join(c, "Info.plist"), "utf8")) as Record<string, PlistValue>;
    expect(existsSync(join(c, "MacOS", info.CFBundleExecutable as string))).toBe(true);
    expect(existsSync(join(c, "Resources", `${info.CFBundleIconFile}.icns`))).toBe(true);
    if (process.platform !== "win32") expect(statSync(join(c, "MacOS", "Hello")).mode & 0o777).toBe(0o755);
    // Where the host will look for its payload is where it was put.
    expect(bundlePayloadPath(r.executable)).toBe(join(c, "Resources", "app.vnpk"));
    // A packaged executable is not a bare host: asking it for host info would launch the app.
    expect(isStamped(r.executable)).toBe(true);
    expect(isStamped(host)).toBe(false);
  });

  test("a rebuild replaces the bundle whole; a failed one leaves the old bundle and no .partial", () => {
    const d = fresh("rebuild");
    const host = fakeMachO(join(d, "vn-app"));
    const out = join(d, "Hello.app");
    write(out, host);
    writeFileSync(join(out, "Contents", "Resources", "stale.txt"), "x");
    write(out, host, new Uint8Array([9]));
    expect(existsSync(join(out, "Contents", "Resources", "stale.txt"))).toBe(false);
    expect([...readFileSync(join(out, "Contents", "Resources", "app.vnpk"))]).toEqual([9]);
    expect(() => write(out, join(d, "no-such-host"))).toThrow();
    expect([...readFileSync(join(out, "Contents", "Resources", "app.vnpk"))]).toEqual([9]);
    expect(existsSync(`${out}.partial`)).toBe(false);
    expect(() => writeAppBundle({ host, out, executable: "a/b", payload: new Uint8Array(), infoPlist: "" })).toThrow(/file name/);
  });
});

// ---------------------------------------------------------------------------
// signing
// ---------------------------------------------------------------------------

describe("codesign", () => {
  test("ad hoc is just --sign -; an identity adds the hardened runtime, a timestamp and the entitlements", () => {
    expect(codesignArgs("A.app", AD_HOC, "e.plist")).toEqual(["--force", "--sign", "-", "A.app"]);
    expect(codesignArgs("A.app", "Developer ID Application: X (T)", "e.plist")).toEqual([
      "--force",
      "--options",
      "runtime",
      "--timestamp",
      "--entitlements",
      "e.plist",
      "--sign",
      "Developer ID Application: X (T)",
      "A.app",
    ]);
  });

  test("the plan: off, off-Mac (with the command to run), no codesign, and the ad-hoc default", () => {
    const base = { app: "/o/A.app", entitlements: "/o/.vn/entitlements.plist" };
    expect(signPlan({ ...base, want: false, codesign: "/usr/bin/codesign", onMac: true })).toEqual({
      identity: null,
      note: "not signed (signing is off)",
    });
    const off = signPlan({ ...base, want: "Developer ID Application: X (T)", codesign: null, onMac: false });
    expect(off.identity).toBeNull();
    expect(off.note).toContain("--entitlements /o/.vn/entitlements.plist --sign 'Developer ID Application: X (T)' /o/A.app");
    expect(signPlan({ ...base, want: undefined, codesign: null, onMac: false }).note).toContain("codesign --force --sign - /o/A.app");
    expect(signPlan({ ...base, want: undefined, codesign: null, onMac: true }).note).toMatch(/not on PATH/);
    expect(() => signPlan({ ...base, want: "X", codesign: null, onMac: true })).toThrow(/xcode-select/);
    expect(signPlan({ ...base, want: undefined, codesign: "/usr/bin/codesign", onMac: true })).toEqual({ identity: "-", note: null });
    expect(signPlan({ ...base, want: "X", codesign: "/usr/bin/codesign", onMac: true })).toEqual({ identity: "X", note: null });
  });
});

// ---------------------------------------------------------------------------
// vn.config.json "macos"
// ---------------------------------------------------------------------------

describe('vn.config.json "macos"', () => {
  function app(config: Record<string, unknown>, files: Record<string, Uint8Array | string> = {}): string {
    const root = fresh("app");
    mkdirSync(join(root, "src"));
    writeFileSync(join(root, "src", "main.ts"), "");
    writeFileSync(join(root, "vn.config.json"), JSON.stringify({ name: "t", version: "1.0.0", ...config }));
    for (const [k, v] of Object.entries(files)) writeFileSync(join(root, k), v);
    return root;
  }

  test("defaults: the bundle id from id (no '_'), the name from productName, the icon and copyright from windows", () => {
    const m = loadAppConfig(
      app(
        { id: "gl.argon.my_app", productName: "My App", windows: { icon: "i.png", copyright: "© X", exeName: "myapp-win" } },
        { "i.png": encodePng(solid(16)) },
      ),
    ).macos;
    expect(m.bundleId).toBe("gl.argon.my-app");
    expect(m.bundleIdFrom).toBe("id");
    expect(m.exeName).toBe("My App");
    expect(m.iconPath).toMatch(/i\.png$/);
    expect(m.copyright).toBe("© X");
    expect(m.minimumSystemVersion).toBe(MACOS_MINIMUM);
    expect(m.category).toBeNull();
    expect(m.sign).toBeUndefined();
  });

  test("every key, and a short category made the full UTI", () => {
    const m = loadAppConfig(
      app(
        {
          macos: {
            bundleId: "com.example.notes",
            exeName: "Notes",
            icon: "m.png",
            category: "productivity",
            minimumSystemVersion: "14.2",
            copyright: "© Y",
            microphoneUsageDescription: "talk",
            cameraUsageDescription: "see",
            sign: "-",
            $comment: "ignored",
          },
        },
        { "m.png": encodePng(solid(16)) },
      ),
    ).macos;
    expect(m).toMatchObject({
      bundleId: "com.example.notes",
      bundleIdFrom: "macos.bundleId",
      exeName: "Notes",
      category: "public.app-category.productivity",
      minimumSystemVersion: "14.2",
      copyright: "© Y",
      microphoneUsageDescription: "talk",
      cameraUsageDescription: "see",
      sign: "-",
    });
    expect(loadAppConfig(app({ macos: { sign: false } })).macos.sign).toBe(false);
  });

  test("mistakes are build errors on every platform", () => {
    const bad = (macos: unknown, re: RegExp) => expect(() => loadAppConfig(app({ macos }))).toThrow(re);
    bad({ icno: "x.png" }, /unknown key "icno"/);
    bad([], /must be an object/);
    bad({ bundleId: "notes" }, /not a bundle identifier/);
    bad({ bundleId: "com.example.my_app" }, /not a bundle identifier/);
    bad({ category: "Games!" }, /not an app category/);
    bad({ minimumSystemVersion: "Ventura" }, /not a version/);
    bad({ sign: true }, /codesign identity/);
    bad({ microphoneUsageDescription: 3 }, /must be a string/);
    bad({ icon: "missing.png" }, /does not exist/);
    expect(() => loadAppConfig(app({ macos: {} }))).not.toThrow(ConfigError);
  });

  test("a minimum below the host's is a warning; the no-icon warning names the platform asked about", () => {
    expect(loadAppConfig(app({ macos: { minimumSystemVersion: "12" } })).warnings.join("\n")).toMatch(/below 13\.0/);
    const root = app({});
    expect(loadAppConfig(root).warnings.join("\n")).toMatch(/"windows\.icon".*default Windows icon/);
    expect(loadAppConfig(root, { platform: "macos" }).warnings.join("\n")).toMatch(/"macos\.icon".*generic macOS icon/);
  });
});

// ---------------------------------------------------------------------------
// hosts per platform
// ---------------------------------------------------------------------------

describe("hosts per platform", () => {
  test("the Windows row is the paths vn build always used", () => {
    const w = hostsFor("windows", "x64");
    expect(repoHostExe("R", w, "release")).toBe(join("R", "target", "x86_64-pc-windows-msvc", "release", "vn-app.exe"));
    expect(w.cargoTarget).toEqual([]);
    if (process.env.LOCALAPPDATA) {
      expect(cachedHost("1.2.3", "standard", "windows")).toBe(
        join(process.env.LOCALAPPDATA, "vue-native", "hosts", "1.2.3", "standard", "vn-host.exe"),
      );
    }
  });

  test("macOS: the triple follows the CPU; the cache lives in ~/Library/Caches", () => {
    expect(hostsFor("macos", "arm64").triple).toBe("aarch64-apple-darwin");
    expect(hostsFor("macos", "x64").triple).toBe("x86_64-apple-darwin");
    const m = hostsFor("macos", "arm64");
    expect(repoHostExe("R", m, "debug")).toBe(join("R", "target", "aarch64-apple-darwin", "debug", "vn-app"));
    expect(m.cargoTarget).toEqual(["--target", "aarch64-apple-darwin"]);
    expect(cachedHost("1.2.3", "no-voice", "macos")).toMatch(/Library[\\/]Caches[\\/]vue-native[\\/]hosts[\\/]1\.2\.3[\\/]no-voice[\\/]vn-host$/);
  });

  test("--platform spellings", () => {
    expect(parsePlatform("darwin")).toBe("macos");
    expect(parsePlatform("MacOS")).toBe("macos");
    expect(parsePlatform("win32")).toBe("windows");
    expect(() => parsePlatform("linux")).toThrow(ConfigError);
  });

  const other = currentPlatform() === "macos" ? "windows" : "macos";
  describe.skipIf(other !== "macos")("a macOS host from another OS", () => {
    test("must be named, must be a Mach-O, and is described by host.json when there is one", () => {
      const d = fresh("cross");
      expect(() => resolveHost({ variant: "standard", platform: "macos" })).toThrow(/needs the host named/);
      writeFileSync(join(d, "vn-app.exe"), new Uint8Array([0x4d, 0x5a, 0, 0, 0, 0, 0, 0]));
      expect(() => resolveHost({ variant: "standard", platform: "macos", host: join(d, "vn-app.exe") })).toThrow(/not a Mach-O/);

      const host = fakeMachO(join(d, "vn-app"));
      const trusted = resolveHost({ variant: "no-voice", platform: "macos", host });
      expect(trusted.info.variant).toBe("no-voice");
      expect(trusted.info.features).not.toContain("voice");
      expect(trusted.info.features).toContain("webgpu");
      expect(trusted.unverified).toMatch(/cannot be run/);

      const info = { ...crossHostInfo(host, "standard", "macos").info, crate: "9.9.9", profile: "release", vueNative: "0.0.0-other" };
      writeFileSync(join(d, "host.json"), JSON.stringify(info));
      expect(() => resolveHost({ variant: "standard", platform: "macos", host })).toThrow(/0\.0\.0-other/);
      const mismatchAllowed = resolveHost({ variant: "standard", platform: "macos", host, allowMismatch: true });
      expect(mismatchAllowed.info.profile).toBe("release");
      expect(mismatchAllowed.unverified).toMatch(/described by/);
      expect(() => resolveHost({ variant: "no-voice", platform: "macos", host, allowMismatch: true })).toThrow(HostError);
    });

    test("vn build --platform macos --host <fake>: a bundle whose app.vnpk is the build's payload", async () => {
      const root = fresh("e2e");
      mkdirSync(join(root, "src"));
      mkdirSync(join(root, "assets"));
      writeFileSync(join(root, "src", "main.ts"), "export const x = 1;\n");
      writeFileSync(join(root, "assets", "a.txt"), "hello");
      writeFileSync(join(root, "icon.png"), encodePng(solid(64)));
      writeFileSync(
        join(root, "vn.config.json"),
        JSON.stringify({
          name: "e2e",
          productName: "E2E & Co",
          version: "2.3.4-beta.1",
          singleInstance: true,
          protocols: ["e2e"],
          windows: { icon: "icon.png" },
          macos: { bundleId: "com.example.e2e", microphoneUsageDescription: "mic" },
        }),
      );
      const host = fakeMachO(join(fresh("e2e-host"), "vn-app"));
      const out = join(root, "dist");
      const r = await buildApp({ target: root, out, host, platform: "macos", log: () => {} });

      expect(r.platform).toBe("macos");
      expect(r.stamp).toBeNull();
      const app = join(out, "E2E & Co.app");
      expect(r.bundle!.app).toBe(app);
      expect(r.exe).toBe(join(app, "Contents", "MacOS", "E2E & Co"));
      expect(r.bundle!.signed).toBeNull();
      expect(r.bundle!.signNote).toMatch(/codesign runs on macOS only/);
      expect(r.warnings.join("\n")).toMatch(/cannot be run/);

      const shipped = readFileSync(join(app, "Contents", "Resources", "app.vnpk"));
      expect(shipped.equals(readFileSync(r.vnpk))).toBe(true);
      const payload = readVnpk(new Uint8Array(shipped));
      expect(payload.header.dataOff % 4096).toBe(0);
      const meta = JSON.parse(new TextDecoder().decode(payload.bytes(payload.find(KIND.META, NAMES.META)!)));
      expect(meta).toMatchObject({ name: "e2e", productName: "E2E & Co", version: "2.3.4-beta.1" });
      expect(payload.find(KIND.ASSET, "assets/a.txt")).toBeDefined();

      const info = readPlist(readFileSync(join(app, "Contents", "Info.plist"), "utf8")) as Record<string, PlistValue>;
      expect(info).toMatchObject({
        CFBundleIdentifier: "com.example.e2e",
        CFBundleName: "E2E & Co",
        CFBundleExecutable: "E2E & Co",
        CFBundleShortVersionString: "2.3.4",
        CFBundleVersion: "2.3.4",
        CFBundleIconFile: "AppIcon",
        NSMicrophoneUsageDescription: "mic",
        CFBundleURLTypes: [{ CFBundleURLName: "com.example.e2e", CFBundleURLSchemes: ["e2e"] }],
      });
      expect(parseIcns(new Uint8Array(readFileSync(join(app, "Contents", "Resources", "AppIcon.icns")))).length).toBe(4);
      const ent = readPlist(readFileSync(r.bundle!.entitlements, "utf8"));
      expect(ent).toEqual({ "com.apple.security.cs.allow-jit": true, "com.apple.security.device.audio-input": true });
    }, 120_000);
  });
});
