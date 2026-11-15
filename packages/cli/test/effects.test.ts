import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deflateRawSync, gzipSync } from "node:zlib";
import { effectFindings } from "../check";
import { watchAppEffects } from "../cli/effects";
import { compileAppEffects, EffectsError, SCHEMA, writeEffectFiles } from "../effects/compile";
import { resolveCompiler, unpack } from "../effects/redslangc";
import { FIXTURES } from "./check-fixtures";

const scratch = mkdtempSync(join(tmpdir(), "sk-effects-"));
afterAll(() => rmSync(scratch, { recursive: true, force: true }));

const enc = new TextEncoder();

function tar(files: Record<string, string>): Uint8Array<ArrayBuffer> {
  const blocks: Uint8Array[] = [];
  for (const [name, text] of Object.entries(files)) {
    const header = new Uint8Array(512);
    header.set(enc.encode(name), 0);
    const body = enc.encode(text);
    header.set(enc.encode(`${body.length.toString(8).padStart(11, "0")}\0`), 124);
    header[156] = "0".charCodeAt(0);
    header.set(enc.encode("ustar\0"), 257);
    blocks.push(header, body, new Uint8Array((512 - (body.length % 512)) % 512));
  }
  blocks.push(new Uint8Array(1024));
  return new Uint8Array(gzipSync(Buffer.concat(blocks)));
}

function zip(files: Record<string, string>): Uint8Array<ArrayBuffer> {
  const locals: Uint8Array[] = [];
  const central: Uint8Array[] = [];
  let offset = 0;
  for (const [name, text] of Object.entries(files)) {
    const n = enc.encode(name);
    const data = new Uint8Array(deflateRawSync(enc.encode(text)));
    const local = new Uint8Array(30 + n.length);
    const lv = new DataView(local.buffer);
    lv.setUint32(0, 0x04034b50, true);
    lv.setUint16(8, 8, true);
    lv.setUint32(18, data.length, true);
    lv.setUint16(26, n.length, true);
    local.set(n, 30);
    const entry = new Uint8Array(46 + n.length);
    const cv = new DataView(entry.buffer);
    cv.setUint32(0, 0x02014b50, true);
    cv.setUint16(10, 8, true);
    cv.setUint32(20, data.length, true);
    cv.setUint16(28, n.length, true);
    cv.setUint32(42, offset, true);
    entry.set(n, 46);
    locals.push(local, data);
    central.push(entry);
    offset += local.length + data.length;
  }
  const size = central.reduce((s, c) => s + c.length, 0);
  const end = new Uint8Array(22);
  const ev = new DataView(end.buffer);
  ev.setUint32(0, 0x06054b50, true);
  ev.setUint16(10, central.length, true);
  ev.setUint32(12, size, true);
  ev.setUint32(16, offset, true);
  return new Uint8Array(Buffer.concat([...locals, ...central, end]));
}

describe("the release archives", () => {
  const files = { "redslangc-v0/redslangc": "binary", "redslangc-v0/licenses/slang.txt": "Apache", "._x": "mac" };
  test("a tar.gz unpacks to its files without the top directory", () => {
    const out = unpack(tar(files), "redslangc.tar.gz").map((f) => [f.name, new TextDecoder().decode(f.data)]);
    expect(out).toEqual([
      ["redslangc", "binary"],
      ["licenses/slang.txt", "Apache"],
    ]);
  });

  test("a zip unpacks the same way", () => {
    const out = unpack(zip({ "redslangc-v0/redslangc.exe": "exe", "redslangc-v0/../evil": "x" }), "r.zip");
    expect(out.map((f) => f.name)).toEqual(["redslangc.exe"]);
  });
});

// The compiler: cached, named by SK_REDSLANGC, or downloaded (CI runs `bun run shaders:check` first).
const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;

function app(files: Record<string, string>): string {
  const root = mkdtempSync(join(scratch, "app-"));
  mkdirSync(join(root, "shaders"));
  for (const [name, text] of Object.entries(files)) writeFileSync(join(root, "shaders", name), text);
  return root;
}

const RIPPLE = `import sk;

[SkEffect("ripple")]
struct Ripple : IEffect
{
    [SkAnimated] [SkDefault("1.5")] float speed;
    [SkColor] [SkOptional] [SkDefault("#7fd4ff")] float4 tint;

    float4 shade(EffectInput input)
    {
        return sk_premultiply(tint * input.coverage);
    }
};
`;

describe.skipIf(!compiler)("an app's effects", () => {
  test("compile to every target and a schema-7 manifest that names them", async () => {
    const root = app({ "ripple.slang": RIPPLE });
    const built = await compileAppEffects(root, compiler!);
    expect(built.files.map((f) => f.name)).toEqual(["ripple.hlsl", "ripple.wgsl", "ripple.metal", "effects.json"]);
    const manifest = JSON.parse(built.files.at(-1)!.text);
    expect(manifest.schema).toBe(SCHEMA);
    expect(manifest.kind).toBe("user");
    expect(manifest.effects[0]).toMatchObject({
      name: "ripple",
      hlsl: "ripple.hlsl",
      wgsl: "ripple.wgsl",
      metal: "ripple.metal",
      source: "shaders/ripple.slang",
      signature: "ripple(<number:speed>, <color:tint>?)",
      animatedParam: 0,
    });
    const metal = built.files.find((f) => f.name === "ripple.metal")!.text;
    expect(metal).toContain("[[fragment]] pixelOutput_0 effect_fragment(");
    expect(metal).toContain("[[buffer(1)]]");
    expect(metal).toContain('#line 4 "shaders/ripple.slang+wrapper"');
    expect(metal).not.toContain(root.replace(/\\/g, "/"));

    const out = join(root, "out");
    mkdirSync(out);
    writeFileSync(join(out, "gone.hlsl"), "");
    writeEffectFiles(out, built.files);
    expect(JSON.parse(readFileSync(join(out, "effects.json"), "utf8")).effects).toHaveLength(1);
    expect(() => readFileSync(join(out, "gone.hlsl"))).toThrow();
  });

  test("a syntax error is reported at the author's line, every broken effect at once", async () => {
    const root = app({
      "ripple.slang": RIPPLE.replace("return sk_premultiply", "return sk_premultiply_nope"),
      "wave.slang": RIPPLE.replace('"ripple"', '"wave"').replace("float speed;", "float speed"),
    });
    const error = await compileAppEffects(root, compiler!).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(EffectsError);
    const problems = (error as EffectsError).problems.filter((p) => p.severity === "error");
    const ripple = problems.find((p) => p.file.endsWith("ripple.slang"));
    expect(ripple).toMatchObject({ line: 11 });
    expect(problems.find((p) => p.file.endsWith("wave.slang"))).toMatchObject({ line: 7 });
  });

  test("sk check reports a shader that does not compile at its line", async () => {
    const findings = await effectFindings(join(FIXTURES, "check-shaders"));
    expect(findings.map((f) => [f.code, f.range.start.line])).toEqual([["SK0005", 6]]);
  });
});

test.skipIf(!compiler)(
  "sk dev's watcher rebuilds on a change and keeps the last good build on an error",
  async () => {
    const root = app({ "ripple.slang": RIPPLE });
    const out = join(root, "out");
    const events: string[] = [];
    let wake = () => {};
    const next = () => new Promise<void>((resolve) => (wake = resolve));
    const watch = watchAppEffects({
      appRoot: root,
      out,
      compiler: compiler!,
      debounceMs: 30,
      onBuilt: (effects) => {
        events.push(`built ${effects.compiled.map((c) => c.effect.signature).join()}`);
        wake();
      },
      onError: (text) => {
        events.push(`error ${text}`);
        wake();
      },
    });
    try {
      let step = next();
      writeFileSync(join(root, "shaders", "ripple.slang"), RIPPLE.replace("float speed;", "float speed; float depth;"));
      await step;
      expect(events.at(-1)).toBe("built ripple(<number:speed>, <number:depth>, <color:tint>?)");
      const good = readFileSync(join(out, "effects.json"), "utf8");
      step = next();
      writeFileSync(join(root, "shaders", "ripple.slang"), RIPPLE.replace("float4 shade", "float4 shade("));
      await step;
      expect(events.at(-1)).toMatch(/^error \S*shaders[\\/]ripple\.slang:9:18: error: /);
      expect(readFileSync(join(out, "effects.json"), "utf8")).toBe(good);
    } finally {
      watch.close();
    }
  },
  20_000,
);
