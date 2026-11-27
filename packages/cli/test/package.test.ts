import { afterAll, describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { inflateRawSync } from "node:zlib";
import { keyRange, packageFindings } from "../check/package";
import { loadAppConfig } from "../cli/config";
import { encodePng } from "../cli/icon";
import { BLOCK, contentTypesXml, encodeFile, writeAppx, zipName } from "../cli/package/appx";
import { packageApp } from "../cli/package/command";
import { packageProblems, parsePackageConfig } from "../cli/package/config";
import { appInstallerXml, appxManifest, msixAssets, parsePublisher, payloadFiles } from "../cli/package/msix";
import { resourcesPri } from "../cli/package/pri";

// Made up, in the shape Windows prints a real code-signing subject: E= first, quoted values with commas.
const SUBJECT = 'E=support@example.com, CN="Example Widgets, LLC", O="Example Widgets, LLC", L=Springfield, C=US';

const dirs: string[] = [];
afterAll(() => dirs.forEach((d) => rmSync(d, { recursive: true, force: true })));
const tmp = () => {
  const d = mkdtempSync(join(tmpdir(), "sk-package-"));
  dirs.push(d);
  return d;
};

const square = (n: number) => encodePng({ width: n, height: n, data: new Uint8Array(n * n * 4).fill(200) });

function app(config: Record<string, unknown>) {
  const dir = tmp();
  writeFileSync(join(dir, "main.ts"), "");
  writeFileSync(join(dir, "icon.png"), square(64));
  writeFileSync(join(dir, "sk.config.json"), JSON.stringify({ name: "notes", version: "1.4.2", ...config }));
  return loadAppConfig(dir, { platform: "windows" });
}

const msixConfig = (extra: Record<string, unknown> = {}) => ({
  id: "com.example.notes",
  productName: "Notes",
  protocols: ["notes"],
  singleInstance: true,
  package: {
    publisher: "Example Widgets",
    icon: "icon.png",
    fileAssociations: [{ extensions: [".Note"], name: "Notes document", mimeType: "application/x-notes" }],
    windows: { msix: { publisher: SUBJECT, architecture: "x64", capabilities: ["microphone", "internetClient"] } },
    ...extra,
  },
});

/** The entries of a ZIP64 package as its central directory lists them. */
function readZip(bytes: Buffer) {
  const eocd64 = Number(bytes.readBigUInt64LE(bytes.length - 22 - 20 + 8));
  const count = Number(bytes.readBigUInt64LE(eocd64 + 32));
  let o = Number(bytes.readBigUInt64LE(eocd64 + 48));
  const entries: { name: string; method: number; crc: number; size: number; packed: number; offset: number }[] = [];
  for (let i = 0; i < count; i++) {
    expect(bytes.readUInt32LE(o)).toBe(0x02014b50);
    const nameLength = bytes.readUInt16LE(o + 28);
    const extra = o + 46 + nameLength;
    expect(bytes.readUInt16LE(extra)).toBe(1);
    entries.push({
      name: bytes.subarray(o + 46, extra).toString(),
      method: bytes.readUInt16LE(o + 10),
      crc: bytes.readUInt32LE(o + 16),
      size: Number(bytes.readBigUInt64LE(extra + 4)),
      packed: Number(bytes.readBigUInt64LE(extra + 12)),
      offset: Number(bytes.readBigUInt64LE(extra + 20)),
    });
    o = extra + bytes.readUInt16LE(o + 30) + bytes.readUInt16LE(o + 32);
  }
  const data = (e: (typeof entries)[number]) => {
    const start = e.offset + 30 + bytes.readUInt16LE(e.offset + 26) + bytes.readUInt16LE(e.offset + 28);
    const body = bytes.subarray(start, start + e.packed);
    return e.method === 8 ? inflateRawSync(body) : body;
  };
  return { entries, data };
}

describe("the MSIX publisher", () => {
  test("a subject with E=, quoted values holding commas and an OID parses into its attributes", () => {
    const parsed = parsePublisher(`${SUBJECT}, OID.2.5.4.97=VATUS-123, OU="say ""hi"""`);
    expect("rdns" in parsed && parsed.rdns).toEqual([
      { type: "E", value: "support@example.com" },
      { type: "CN", value: "Example Widgets, LLC" },
      { type: "O", value: "Example Widgets, LLC" },
      { type: "L", value: "Springfield" },
      { type: "C", value: "US" },
      { type: "OID.2.5.4.97", value: "VATUS-123" },
      { type: "OU", value: 'say "hi"' },
    ]);
  });

  test("what the manifest schema refuses is refused", () => {
    for (const bad of [
      "CN=Example,O=Example",
      "O=Example, Ltd.",
      "CN=",
      'CN="Example',
      "X=1",
      "cn=Example",
      "CN=a#b",
      "",
    ]) {
      expect("error" in parsePublisher(bad)).toBe(true);
    }
  });

  test("the manifest and the update feed carry the subject byte for byte", () => {
    const config = parsePackageConfig(app(msixConfig())).config;
    const manifest = appxManifest({
      config,
      executable: "Notes.exe",
      assets: { storeLogo: "a\\s.png", square150: "a\\150.png", square44: "a\\44.png", wide310: null },
      fileTypeLogos: [null],
    });
    const unescape = (s: string) => s.replace(/&quot;/g, '"').replace(/&amp;/g, "&");
    expect(unescape(/<Identity [^>]*Publisher="([^"]*)"/.exec(manifest)![1]!)).toBe(SUBJECT);
    const feed = appInstallerXml(
      {
        ...config,
        windows: {
          ...config.windows,
          msix: {
            ...config.windows.msix,
            appInstaller: {
              url: "https://x.test/n",
              hoursBetweenUpdateChecks: 0,
              showPrompt: false,
              updateBlocksActivation: false,
            },
          },
        },
      },
      "n.msix",
      "n.appinstaller",
    )!;
    expect(unescape(/<MainPackage [^>]*Publisher="([^"]*)"/.exec(feed)![1]!)).toBe(SUBJECT);
    expect(feed).toContain('Uri="https://x.test/n/n.msix"');
  });
});

describe("the MSIX manifest", () => {
  const config = parsePackageConfig(app(msixConfig())).config;
  const manifest = appxManifest({
    config,
    executable: "bin\\Notes.exe",
    assets: { storeLogo: "a\\s.png", square150: "a\\150.png", square44: "a\\44.png", wide310: null },
    fileTypeLogos: [null],
  });

  test("a protocol launches the exe the way the host reads a protocol launch", () => {
    expect(manifest).toContain('<uap3:Protocol Name="notes" Parameters="--sk-protocol-launch -- &quot;%1&quot;">');
    expect(manifest).toContain('Executable="bin\\Notes.exe" EntryPoint="Windows.FullTrustApplication"');
  });

  test("Action Center starts the same executable and resolves the same COM class across updates", () => {
    const clsid = /ToastActivatorCLSID="([a-f0-9-]{36})"/.exec(manifest)?.[1];
    expect(clsid).toBeDefined();
    expect(manifest).toContain(`<com:Class Id="${clsid}"`);
    expect(manifest).toContain('<com:ExeServer Executable="bin\\Notes.exe" Arguments="--sk-toast-activation"');
    const next = appxManifest({
      config: { ...config, identity: { ...config.identity, version: "2.0.0" } },
      executable: "new.exe",
      assets: { storeLogo: "a.png", square150: "b.png", square44: "c.png", wide310: null },
      fileTypeLogos: [null],
    });
    expect(next).toContain(`ToastActivatorCLSID="${clsid}"`);
  });

  test("file types are listed with the dot, lowercased, and the file path as the argument", () => {
    expect(manifest).toContain('<uap:FileType ContentType="application/x-notes">.note</uap:FileType>');
    expect(manifest).toContain('Parameters="&quot;%1&quot;"');
  });

  test("capabilities come in the order the schema requires, runFullTrust always", () => {
    const order = [...manifest.matchAll(/<(\w+:)?(Capability|DeviceCapability) Name="(\w+)"/g)].map((m) => m[3]);
    expect(order).toEqual(["internetClient", "runFullTrust", "microphone"]);
  });

  test("the version gets its fourth part", () => {
    expect(manifest).toContain('Version="1.4.2.0"');
  });
});

describe("the block map", () => {
  const data = new Uint8Array(BLOCK * 2 + 1234);
  for (let i = 0; i < data.length; i++) data[i] = (i * 31) % 251 < 40 ? (i * 7) & 255 : 65;

  test("a deflated file is compressed block by block, each block's hash over its uncompressed bytes", () => {
    const { body, entry } = encodeFile({ path: "dir/a b.bin", data }, true);
    expect(entry.blocks.length).toBe(3);
    expect(entry.blocks.reduce((n, b) => n + b.size!, 0)).toBe(body.length);
    let at = 0;
    entry.blocks.forEach((b, i) => {
      const slice = data.subarray(i * BLOCK, (i + 1) * BLOCK);
      expect(b.hash).toBe(createHash("sha256").update(slice).digest("base64"));
      // A block inflates on its own, so an installer can verify it without the blocks before it.
      const alone = Buffer.concat([body.subarray(at, at + b.size!), Buffer.from([3, 0])]);
      expect(Buffer.from(inflateRawSync(alone)).equals(Buffer.from(slice))).toBe(true);
      at += b.size!;
    });
    expect(Buffer.from(inflateRawSync(body)).equals(Buffer.from(data))).toBe(true);
    expect(entry.fileHash).toBe(createHash("sha256").update(data).digest("base64"));
    expect(entry.lfhSize).toBe(30 + "dir/a%20b.bin".length);
  });

  test("a stored file lists hashes without sizes; an empty file lists no block", () => {
    expect(encodeFile({ path: "x.png", data }, false).entry.blocks.every((b) => b.size === null)).toBe(true);
    const empty = encodeFile({ path: "e.txt", data: new Uint8Array(0) }, true);
    expect(empty.entry.blocks).toEqual([]);
    expect(inflateRawSync(empty.body).length).toBe(0);
  });

  test("names outside the unreserved set are percent-encoded as UTF-8", () => {
    expect(zipName("cfg/x(1)!+%ü.txt")).toBe("cfg/x%281%29%21%2B%25%C3%BC.txt");
  });

  test("a file without an extension gets its own content type; an .xml payload does not take the manifest's", () => {
    const types = contentTypesXml(["LICENSE", "cfg/data.XML", "app.exe"]);
    expect(types).toContain('<Override PartName="/LICENSE" ContentType="application/octet-stream" />');
    expect(types).toContain(
      '<Override PartName="/AppxManifest.xml" ContentType="application/vnd.ms-appx.manifest+xml" />',
    );
  });
});

describe("the package file", () => {
  test("entries, sizes, CRCs and local header sizes agree with the block map", () => {
    const dir = tmp();
    const files: Record<string, Uint8Array> = {
      "app.exe": new Uint8Array(BLOCK + 10).fill(7),
      "img/logo.png": square(32),
      "empty.json": new Uint8Array(0),
    };
    const out = join(dir, "t.msix");
    const { blockMap } = writeAppx(out, Object.keys(files).sort(), (p) => files[p]!, "<Package/>");
    const bytes = readFileSync(out);
    const zip = readZip(bytes);
    expect(zip.entries.map((e) => e.name)).toEqual([
      "app.exe",
      "empty.json",
      "img/logo.png",
      "AppxManifest.xml",
      "AppxBlockMap.xml",
      "[Content_Types].xml",
    ]);
    for (const e of zip.entries) {
      const data = zip.data(e);
      expect(data.length).toBe(e.size);
      expect(Bun.hash.crc32(data)).toBe(e.crc);
      expect(bytes.readUInt16LE(e.offset + 6) & 8).toBe(8);
    }
    expect(zip.entries.find((e) => e.name === "img/logo.png")!.method).toBe(0);
    expect(zip.data(zip.entries[4]!).toString()).toBe(blockMap);
    for (const m of blockMap.matchAll(/<File Name="([^"]+)" Size="(\d+)" LfhSize="(\d+)"/g)) {
      const e = zip.entries.find((z) => z.name === m[1]!.replace(/\\/g, "/"))!;
      expect(e.size).toBe(Number(m[2]));
      expect(30 + bytes.readUInt16LE(e.offset + 26)).toBe(Number(m[3]));
    }
  });

  test("the build folder's dot-folders and symbols stay out", () => {
    const dir = tmp();
    mkdirSync(join(dir, ".sk"));
    mkdirSync(join(dir, "cfg"));
    for (const f of [".sk/app.skpk", "Notes.exe", "Notes.pdb", "cfg/media.json", "icu.skpk"])
      writeFileSync(join(dir, f), "x");
    expect(payloadFiles(dir)).toEqual(["Notes.exe", "cfg/media.json", "icu.skpk"]);
  });
});

describe("the MSIX tile images and resources.pri", () => {
  const icon = [{ width: 8, height: 8, data: new Uint8Array(8 * 8 * 4).fill(255) }];
  const sha = (b: Uint8Array) => createHash("sha256").update(b).digest("hex");
  const size = (png: Uint8Array) => {
    const v = new DataView(png.buffer, png.byteOffset);
    return [v.getUint32(16), v.getUint32(20)];
  };

  // The references are `makepri new` (SDK 10.0.26100) over a folder of these names, with the config of
  // `makepri createconfig /dq en-US /pv 10.0.0` minus its <packaging> section, for a package named com.example.notes.
  test("the index is makepri's byte for byte, with the wide tile and file type logos", () => {
    const files = [...msixAssets(icon, true).keys(), "PackageAssets/FileType1.png", "PackageAssets/FileType0.png"];
    expect(sha(resourcesPri("com.example.notes", files))).toBe(
      "e2966fe36f5e4274cd4a3d5e9dce112210c77911c7f73f85ef67fb3dbae833b3",
    );
  });

  test("and without the wide tile", () => {
    expect(sha(resourcesPri("com.example.notes", [...msixAssets([], false).keys()]))).toBe(
      "f51571cbf23b22cf85d055907bbbd3ca0f200e03de41ed5b1ed45b1c70620caa",
    );
  });

  test("a variant the index has no ranking for is refused rather than indexed wrong", () => {
    expect(() => resourcesPri("com.example.notes", ["PackageAssets/Logo.scale-300.png"])).toThrow(/scale-300/);
  });

  test("each variant has the pixel size Windows asks for", () => {
    const assets = msixAssets(icon, true);
    const px = (name: string) => size(assets.get(`PackageAssets/${name}.png`)!);
    expect(px("Square44x44Logo.scale-125")).toEqual([55, 55]);
    expect(px("Square150x150Logo.scale-125")).toEqual([188, 188]);
    expect(px("Wide310x150Logo.scale-125")).toEqual([388, 188]);
    expect(px("StoreLogo.scale-400")).toEqual([200, 200]);
    expect(px("Square44x44Logo.targetsize-24_altform-lightunplated")).toEqual([24, 24]);
    expect(assets.size).toBe(35);
  });
});

describe('sk.config.json "package"', () => {
  test("defaults come from the rest of the config", () => {
    const { config, problems } = parsePackageConfig(
      app({ id: "com.example.notes", protocols: ["notes"], windows: { company: "Example", icon: "icon.png" } }),
    );
    expect(config.identity).toMatchObject({
      displayName: "notes",
      publisher: "Example",
      id: "com.example.notes",
      version: "1.4.2",
    });
    expect(config.protocols).toEqual([{ scheme: "notes", name: "notes" }]);
    expect(config.icons.length).toBe(1);
    expect(config.windows.formats).toEqual(["msix"]);
    expect(problems.filter((p) => p.severity === "error")).toEqual([]);
  });

  test("mistakes are collected, each at its key", () => {
    const { problems } = parsePackageConfig(
      app({
        package: {
          colour: "red",
          fileAssociations: [{ extensions: ["exe"], name: "Programs" }],
          protocols: ["http"],
          windows: { formats: ["dmg"] },
        },
      }),
    );
    expect(problems.filter((p) => p.code === "SK8001").map((p) => p.key)).toEqual([
      "package.colour",
      "package.id",
      "package.fileAssociations[0].extensions",
      "package.protocols[0]",
      "package.windows.formats",
    ]);
  });

  test("MSIX needs a publisher that is a certificate subject", () => {
    const missing = packageProblems(app(msixConfig({ windows: { msix: {} } })), ["windows"]).problems;
    expect(missing.some((p) => p.key === "package.windows.msix.publisher" && p.severity === "error")).toBe(true);
    const naive = packageProblems(app(msixConfig({ windows: { msix: { publisher: "CN=Example,O=Example" } } })), [
      "windows",
    ]).problems;
    expect(naive.some((p) => p.key === "package.windows.msix.publisher" && p.severity === "error")).toBe(true);
    expect(packageProblems(app(msixConfig()), ["windows"]).problems.filter((p) => p.severity === "error")).toEqual([]);
  });

  test("sk check points at the key in sk.config.json", () => {
    const text = '{\n  "package": {\n    "windows": { "msix": { "publisher": "x" } }\n  }\n}';
    expect(keyRange(text, "package.windows.msix.publisher").start).toEqual({ line: 2, character: 27 });
    const config = app({ package: { windows: { msix: {} } } });
    expect(packageFindings(config, "sk.config.json", text).map((f) => f.code)).toContain("SK8001");
  });
});

describe("sk package --from", () => {
  test("writes an MSIX of the folder and refuses a config with errors before writing anything", async () => {
    const config = app(msixConfig());
    const build = join(config.appRoot, "out");
    mkdirSync(build);
    writeFileSync(join(build, "Notes.exe"), "MZ");
    const out = join(config.appRoot, "pkg");
    const result = await packageApp({
      target: config.appRoot,
      platform: "windows",
      from: build,
      exe: "Notes.exe",
      out,
    });
    expect(result.artifacts.map((a) => [a.kind, a.path.slice(out.length + 1), a.signed])).toEqual([
      ["installer", "com.example.notes_1.4.2.0_x64.msix", false],
    ]);
    const zip = readZip(readFileSync(result.artifacts[0]!.path));
    const names = zip.entries.map((e) => e.name);
    expect(names).toContain("resources.pri");
    expect(names).toContain("PackageAssets/Square44x44Logo.targetsize-24_altform-unplated.png");
    const pri = zip.data(zip.entries.find((e) => e.name === "resources.pri")!);
    const assets = names.filter((n) => n.startsWith("PackageAssets/"));
    expect(Buffer.from(pri).equals(Buffer.from(resourcesPri("com.example.notes", assets)))).toBe(true);

    const bad = app({ package: { windows: { msix: {} } } });
    await expect(
      packageApp({ target: bad.appRoot, platform: "windows", from: build, exe: "Notes.exe" }),
    ).rejects.toThrow(/package\.windows\.msix\.publisher/);
  });

  test("a format made only on its own system is refused elsewhere before the build; MSIX is written anywhere", async () => {
    const config = app(msixConfig({ macos: { formats: ["dmg"] } }));
    await expect(packageApp({ target: config.appRoot, platform: "macos", host: "windows" })).rejects.toThrow(
      /dmg package is written on macos, not on windows: run sk package --platform macos there/,
    );
    const build = join(config.appRoot, "out");
    mkdirSync(build);
    writeFileSync(join(build, "Notes.exe"), "MZ");
    const result = await packageApp({
      target: config.appRoot,
      platform: "windows",
      host: "linux",
      from: build,
      exe: "Notes.exe",
      out: join(config.appRoot, "pkg"),
    });
    expect(result.artifacts.map((a) => a.format)).toEqual(["msix"]);
  });
});
