import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import ts from "typescript";
import * as runtime from "../../runtime/src/std/numeric";
import { NumericLowering } from "../numeric";
import { stdModuleFile } from "../runtime-path";

const NAMES = ["i8", "i16", "i32", "u8", "u16", "u32", "i64", "u64", "f16", "f32"] as const;
const IMPORT = `import { ${NAMES.join(", ")} } from "skathi:numeric";\n`;
let serial = 0;

function lower(source: string): { text: string; count: number } {
  const root = join(tmpdir(), "sk-numeric-lowering");
  const pass = new NumericLowering({
    typescript: ts,
    appRoot: root,
    entry: join(root, "main.ts"),
    aliases: { "skathi:numeric": stdModuleFile("skathi:numeric") },
  });
  const text = pass.lower(join(root, `case${serial++}.ts`), source);
  return { text: text ?? source, count: pass.inlined };
}

const transpiler = new Bun.Transpiler({ loader: "ts" });

/** The `run` function of a lowered or original case, with the import replaced by parameters. */
function runner(source: string, bindings: Record<string, unknown>): (...args: unknown[]) => unknown[] {
  const body = transpiler.transformSync(source.replace(/^import[^\n]*\n/, "").replace(/^export /m, ""));
  return new Function(...Object.keys(bindings), `${body}\nreturn run;`)(...Object.values(bindings));
}

const untouchable = Object.fromEntries(
  NAMES.map((name) => [
    name,
    new Proxy(() => {}, {
      get: () => {
        throw new Error(`${name} was not inlined`);
      },
      apply: () => {
        throw new Error(`${name} was not inlined`);
      },
    }),
  ]),
);

const NUMBERS = [
  0,
  -0,
  1,
  -1,
  2,
  7,
  127,
  128,
  -128,
  -129,
  255,
  256,
  32767,
  32768,
  -32768,
  -32769,
  65535,
  65536,
  2 ** 31 - 1,
  2 ** 31,
  -(2 ** 31),
  -(2 ** 31) - 1,
  2 ** 32 - 1,
  2 ** 32,
  2 ** 53,
  -(2 ** 53),
  0.5,
  -0.5,
  1.5,
  -2.5,
  1 / 3,
  65504,
  65520,
  3.4028234663852886e38,
  1e39,
  5e-324,
  1e-8,
  NaN,
  Infinity,
  -Infinity,
];
const COUNTS = [0, 1, 7, 8, 15, 16, 31, 32, 33, -1, 0.5, NaN, 100];
const BIGINTS = [
  0n,
  1n,
  -1n,
  2n,
  2n ** 31n,
  2n ** 63n - 1n,
  -(2n ** 63n),
  2n ** 63n,
  2n ** 64n - 1n,
  2n ** 64n,
  -(2n ** 64n),
  12345678901234567890n,
];

const INT_FORMS = (k: string) => [
  `${k}(a)`,
  `${k}.add(a, b)`,
  `${k}.sub(a, b)`,
  `${k}.mul(a, b)`,
  `${k}.and(a, b)`,
  `${k}.or(a, b)`,
  `${k}.xor(a, b)`,
  `${k}.neg(a)`,
  `${k}.not(a)`,
  `${k}.lt(a, b)`,
  `${k}.shl(a, n)`,
  `${k}.shr(a, n)`,
  `${k}.clz(a)`,
  `${k}.MIN`,
  `${k}.MAX`,
  `${k}.BITS`,
];
const FLOAT_FORMS = (k: string) => [
  `${k}(a)`,
  `${k}.add(a, b)`,
  `${k}.sub(a, b)`,
  `${k}.mul(a, b)`,
  `${k}.div(a, b)`,
  `${k}.fma(a, b, c)`,
];
const BIG_FORMS = (k: string) => [
  `${k}(x)`,
  `${k}.add(x, y)`,
  `${k}.sub(x, y)`,
  `${k}.mul(x, y)`,
  `${k}.and(x, y)`,
  `${k}.or(x, y)`,
  `${k}.xor(x, y)`,
  `${k}.neg(x)`,
  `${k}.not(x)`,
  `${k}.lt(x, y)`,
  `${k}.MIN`,
  `${k}.MAX`,
  `${k}.BITS`,
];

const same = (a: unknown, b: unknown) => Object.is(a, b);

describe("inlined skathi:numeric calls give what the runtime functions give", () => {
  test("every inlined form, over edge values, NaN, -0 and the wrap-around points", () => {
    const forms = [
      ...["i8", "i16", "i32", "u8", "u16", "u32"].flatMap(INT_FORMS),
      ...["f16", "f32"].flatMap(FLOAT_FORMS),
    ];
    const big = ["i64", "u64"].flatMap(BIG_FORMS);
    const source =
      IMPORT +
      `export function run(a: any, b: any, c: any, n: any, x: bigint, y: bigint): unknown[] {\n` +
      `  return x === undefined ? [\n    ${forms.join(",\n    ")},\n  ] : [\n    ${big.join(",\n    ")},\n  ];\n}\n`;
    const lowered = lower(source);
    expect(lowered.count).toBe(forms.length + big.length);
    const original = runner(source, runtime);
    const inlined = runner(lowered.text, untouchable);
    let compared = 0;
    for (const a of NUMBERS)
      for (const b of NUMBERS)
        for (const n of COUNTS) {
          const c = COUNTS[(NUMBERS.indexOf(a) + NUMBERS.indexOf(b)) % COUNTS.length];
          const want = original(a, b, c, n, undefined, undefined);
          const got = inlined(a, b, c, n, undefined, undefined);
          for (let i = 0; i < want.length; i++) {
            if (!same(want[i], got[i]))
              throw new Error(`${forms[i]} with a=${a} b=${b} c=${c} n=${n}: ${want[i]} != ${got[i]}`);
            compared++;
          }
        }
    for (const x of BIGINTS)
      for (const y of BIGINTS) {
        const want = original(0, 0, 0, 0, x, y);
        const got = inlined(0, 0, 0, 0, x, y);
        for (let i = 0; i < want.length; i++) {
          if (!same(want[i], got[i])) throw new Error(`${big[i]} with x=${x} y=${y}: ${want[i]} != ${got[i]}`);
          compared++;
        }
      }
    expect(compared).toBeGreaterThan(1_000_000);
  });
});

describe("sk build", () => {
  const app = mkdtempSync(join(import.meta.dir, "fixtures", "numeric-app-"));
  afterAll(() => rmSync(app, { recursive: true, force: true }));
  mkdirSync(join(app, "src"));
  writeFileSync(join(app, "sk.config.json"), "{}");
  writeFileSync(
    join(app, "src", "main.ts"),
    `import { i32, u8 } from "skathi:numeric";\n` +
      `const a = i32(2147483647);\n` +
      `(globalThis as Record<string, unknown>).out = [i32.add(a, i32(1)), i32.mul(a, a), u8(300), i32.div(i32(7), i32(2))];\n`,
  );
  const run = (...args: string[]) => Bun.spawnSync([process.execPath, ...args], { cwd: join(import.meta.dir, "..") });

  test("the bundle has the calls inlined and computes what the runtime functions compute", () => {
    const out = join(app, "dist", "app.prod.js");
    const result = run("prod.ts", join(app, "src", "main.ts"), out, "--no-minify");
    expect(result.exitCode, result.stderr.toString()).toBe(0);
    const code = readFileSync(out, "utf8");
    expect(code).not.toContain("i32.add(");
    expect(code).toContain("i32.div(");
    const globals: Record<string, unknown> = {};
    new Function("globalThis", code)(globals);
    expect(globals.out).toEqual([-2147483648, 1, 44, 3]);
  });

  test("the module graph for sk check keeps the file as written, so its positions are the file's own", () => {
    const graph = join(app, "graph.json");
    const result = run("prod.ts", join(app, "src", "main.ts"), join(app, "dist", "graph.js"), "--graph", graph);
    expect(result.exitCode, result.stderr.toString()).toBe(0);
    const { bundle } = JSON.parse(readFileSync(graph, "utf8")) as {
      bundle: { code: string; sources: (string | null)[] };
    };
    expect(bundle.sources.map((s) => s && basename(s))).toContain("main.ts");
  });
});

describe("what is inlined", () => {
  const count = (body: string, head = IMPORT) => lower(head + body).count;

  test("nested calls, a namespace import and a renamed import", () => {
    const nested = lower(
      IMPORT + "export function run(a: any, b: any, c: any) { return [i32.add(i32.mul(a, b), i32(u8(c)))]; }\n",
    );
    expect(nested.count).toBe(4);
    const { i32, u8 } = runtime;
    expect(runner(nested.text, untouchable)(70000, 70000, 300)).toEqual([
      i32.add(i32.mul(i32(70000), i32(70000)), i32(u8(300))),
    ]);
    expect(count(`n.i32.add(1, 2); n.u8.MAX;`, `import * as n from "skathi:numeric";\n`)).toBe(2);
    expect(count(`int.add(1, 2); byte(3);`, `import { i32 as int, u8 as byte } from "skathi:numeric";\n`)).toBe(2);
  });

  test("forms that would change a result or evaluate an argument twice stay calls", () => {
    for (const call of [
      "i32.div(a, b)",
      "i32.rem(a, b)",
      "i32.checked(a)",
      "i32.saturating(a)",
      "i32.checkedAdd(a, b)",
      "i32.saturatingAdd(a, b)",
      "i32.overflowingAdd(a, b)",
      "i32.rotl(a, 1)",
      "i32.ctz(a)",
      "i32.popcount(a)",
      "i64.shl(x, 1)",
      "i64(a)",
      "i32.add(a)",
      "i32.add(...pair)",
      "i32.add?.(a, b)",
      "f32.MAX",
    ])
      expect(count(`declare const a: number, b: number, x: bigint, pair: [number, number];\n${call};\n`), call).toBe(0);
  });

  test("a local of the same name is not the module's export, and a local Math keeps Math's forms as calls", () => {
    expect(count("export function f(i32: { add(a: number, b: number): number }) { return i32.add(1, 2); }\n")).toBe(0);
    expect(count("const Math = { imul: () => 0, fround: () => 0 };\ni32.mul(1, 2); f32(1); i32.add(1, 2);\n")).toBe(1);
    expect(count("i32.MAX = 1;\n")).toBe(0);
  });

  test("the lines after a call that spans lines keep their numbers", () => {
    const source = IMPORT + "const v = i32.add(\n  1,\n  2,\n);\nconst w = u8.MAX;\n";
    const { text, count: n } = lower(source);
    expect(n).toBe(2);
    expect(text.split("\n").length).toBe(source.split("\n").length);
    expect(text.split("\n")[5]).toStartWith("const w = ");
  });
});
