/** Vue runtime work under the JIT and under app AOT, measured in benches/aot/app.
 * bun benches/aot/bench.ts [--runs 5] [--variants jit,aot,split-js,split-ts] [--vue-src <vuejs/core checkout>]
 *   [--host <prod+native-aot bare host>] [--compiler <sk-aot.exe>] [--out <dir>]
 *
 *   jit       the AOT build's exe with SK_APP_ENGINE=jit (app.prod.js under the JIT)
 *   aot       `sk build --aot`: the minified production bundle compiled by sk-aot
 *   split-js  the bundle with @vue/runtime-core left out, compiled together with npm's dist ESM of Vue
 *   split-ts  the same, with Vue's TypeScript sources (--vue-src, checked out at node_modules/vue's version)
 */
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { parseArgs } from "node:util";
import { deflateRawSync } from "node:zlib";
import { resolveAotCompiler } from "../../packages/cli/cli/aot-build";
import { buildApp } from "../../packages/cli/cli/build-app";
import { resolveHost } from "../../packages/cli/cli/host";
import { readPayload } from "../../packages/cli/cli/payload";
import { KIND, NAMES, writeSkpk, type SkpkInput } from "../../packages/cli/cli/skpk";
import { stamp } from "../../packages/cli/cli/stamp";
import { nativeManifest, type NativeImage } from "../../packages/cli/aot/native";
import { opaqueHost } from "../../packages/cli/aot/transform";
import { expect, launch, REPO, WORK_DIR } from "../../packages/test/src/index";

const { values } = parseArgs({
  options: {
    runs: { type: "string", default: "5" },
    variants: { type: "string", default: "jit,aot,split-js,split-ts" },
    "vue-src": { type: "string" },
    host: { type: "string" },
    compiler: { type: "string" },
    out: { type: "string" },
  },
});
const runs = Number(values.runs);
if (!Number.isInteger(runs) || runs < 1) throw Error("--runs must be a positive integer");
const VARIANTS = ["jit", "aot", "split-js", "split-ts"] as const;
type VariantName = (typeof VARIANTS)[number];
const variants = values.variants.split(",") as VariantName[];
for (const v of variants) if (!VARIANTS.includes(v)) throw Error(`unknown variant ${v}; known: ${VARIANTS.join(",")}`);
const vueSrc = values["vue-src"] ? resolve(values["vue-src"]) : null;
if (variants.includes("split-ts") && !vueSrc) throw Error("split-ts needs --vue-src <vuejs/core checkout>");

const APP = join(REPO, "benches", "aot", "app");
const OUT = resolve(values.out ?? join(WORK_DIR, "aotbench"));
rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
const log = (line: string) => console.log(`[aotbench] ${line}`);
const sha256 = (bytes: string | Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const posix = (path: string) => path.replaceAll("\\", "/");

const host = resolveHost({ variant: "standard", host: values.host, log });
const aot = host.info.aot;
if (!aot) throw Error(`${host.exe} has no native AOT loader`);
const compiler = await resolveAotCompiler(aot, values.compiler ? resolve(values.compiler) : undefined, log);
log(`host ${host.exe}, sk-aot ${compiler.path}`);

const VUE_PACKAGES = ["runtime-core", "reactivity", "shared"] as const;
const vueDist = (name: string) => join(REPO, "node_modules", "@vue", name, "dist", `${name}.esm-bundler.js`);
const vueVersion: string = JSON.parse(readFileSync(join(REPO, "node_modules", "vue", "package.json"), "utf8")).version;

// The bundle side: Vue switched for a module that reads the namespace the native entry hands over.
const split = join(OUT, "split");
mkdirSync(split, { recursive: true });
const exportsOfCore = Object.keys(await import(vueDist("runtime-core"))).filter((n) => /^[A-Za-z_$][\w$]*$/.test(n));
writeFileSync(
  join(split, "vue-shim.js"),
  `const vue = globalThis.__SK_BENCH_VUE__;\nexport const { ${exportsOfCore.join(", ")} } = vue;\n`,
);
const appConfig = JSON.parse(readFileSync(join(APP, "sk.config.json"), "utf8")) as Record<string, unknown>;
writeFileSync(
  join(split, "sk.config.json"),
  JSON.stringify({ ...appConfig, aliases: { "@vue/runtime-core": join(split, "vue-shim.js") } }, null, 2),
);

const VUE_DEFINES = {
  "process.env.NODE_ENV": '"production"',
  __VUE_OPTIONS_API__: "true",
  __VUE_PROD_DEVTOOLS__: "false",
  __VUE_PROD_HYDRATION_MISMATCH_DETAILS__: "false",
  // What vuejs/core's rollup.config.js sets for the esm-bundler build, in production.
  __DEV__: "false",
  __TEST__: "false",
  __BROWSER__: "false",
  __GLOBAL__: "false",
  __ESM_BUNDLER__: "true",
  __ESM_BROWSER__: "false",
  __CJS__: "false",
  __SSR__: "true",
  __COMPAT__: "false",
  __VERSION__: JSON.stringify(vueVersion),
  __FEATURE_SUSPENSE__: "true",
  __FEATURE_OPTIONS_API__: "true",
  __FEATURE_PROD_DEVTOOLS__: "false",
  __FEATURE_PROD_HYDRATION_MISMATCH_DETAILS__: "false",
};

async function compileSplit(script: string, flavor: "js" | "ts", work: string): Promise<NativeImage & { ms: number }> {
  mkdirSync(work, { recursive: true });
  const paths = Object.fromEntries(
    VUE_PACKAGES.map((name) => [
      `@vue/${name}`,
      [posix(relative(work, flavor === "js" ? vueDist(name) : join(vueSrc!, "packages", name, "src", "index.ts")))],
    ]),
  );
  writeFileSync(
    join(work, "tsconfig.json"),
    JSON.stringify({
      compilerOptions: {
        target: "esnext",
        module: "esnext",
        moduleResolution: "bundler",
        strict: true,
        skipLibCheck: true,
        useDefineForClassFields: false,
        lib: ["esnext", "dom"],
        types: [],
        paths,
      },
      files: [
        "native-input.js",
        ...(flavor === "ts" ? [posix(relative(work, join(vueSrc!, "packages", "global.d.ts")))] : []),
      ],
    }),
  );
  const input = [
    'import * as vue from "@vue/runtime-core";',
    "__SK_AOT_HOST__.__SK_BENCH_VUE__ = vue;",
    opaqueHost(script),
  ].join("\n");
  writeFileSync(join(work, "native-input.js"), input);
  const defines = Object.entries(VUE_DEFINES).flatMap(([k, v]) => ["--define", `${k}=${v}`]);
  const t0 = performance.now();
  const child = Bun.spawnSync([compiler.path, "compile", "native-input.js", "app.ir", "app.image", ...defines], {
    cwd: work,
  });
  const ms = Math.round(performance.now() - t0);
  if (child.exitCode !== 0)
    throw Error(`sk-aot (${flavor}) failed:\n${child.stderr.toString()}${child.stdout.toString()}`);
  const image = new Uint8Array(readFileSync(join(work, "app.image")));
  const entries = readFileSync(join(work, "app.image.entries"), "utf8").trim().split(/\s+/).map(Number);
  if (entries.length !== 1) throw Error(`sk-aot reported entries ${entries.join(",")}`);
  return {
    image,
    entry: entries[0]!,
    identity: readFileSync(join(work, "app.image.identity")).toString("hex"),
    imageSha256: sha256(image),
    sourceBundleSha256: sha256(script),
    nativeInputSha256: sha256(input),
    irSha256: sha256(readFileSync(join(work, "app.ir"))),
    diagnostics: child.stdout.toString().trim(),
    uses: null,
    ms,
  };
}

interface Built {
  exe: string;
  name: string;
  env: Record<string, string>;
  imageBytes: number | null;
}
const built = new Map<VariantName, Built>();
const name = String(appConfig.name);

if (variants.includes("jit") || variants.includes("aot")) {
  const result = await buildApp({
    target: APP,
    out: join(OUT, "aot"),
    host: host.exe,
    variant: "standard",
    aot: true,
    aotCompiler: compiler.path,
    log,
  });
  const imageBytes = result.native?.imageBytes ?? null;
  built.set("aot", { exe: result.exe!, name, env: {}, imageBytes });
  built.set("jit", { exe: result.exe!, name, env: { SK_APP_ENGINE: "jit" }, imageBytes: null });
  log(
    `aot: image ${((imageBytes ?? 0) / 2 ** 20).toFixed(1)} MiB, bundle ${(result.bundleBytes / 1024).toFixed(0)} KiB`,
  );
}

const splitFlavors = (["js", "ts"] as const).filter((f) => variants.includes(`split-${f}`));
if (splitFlavors.length) {
  const result = await buildApp({
    target: APP,
    configPath: join(split, "sk.config.json"),
    out: join(split, "payload"),
    host: host.exe,
    variant: "standard",
    aot: false,
    payloadOnly: true,
    log,
  });
  const payload = readPayload(new Uint8Array(readFileSync(result.skpk)));
  const scriptEntry = payload.find(KIND.SCRIPT, NAMES.SCRIPT);
  if (!scriptEntry) throw Error("the split payload has no script");
  const script = new TextDecoder().decode(payload.bytes(scriptEntry));
  if (script.includes("__vccOpts")) throw Error("the split bundle still holds @vue/runtime-core");
  const kept: SkpkInput[] = payload.entries.map((e) => ({
    name: e.name,
    kind: e.kind,
    data: payload.bytes(e).slice(),
    ...(e.alignLog2 === undefined ? {} : { alignLog2: e.alignLog2 }),
  }));
  for (const flavor of splitFlavors) {
    const native = await compileSplit(script, flavor, join(split, flavor));
    const manifest = JSON.stringify(
      nativeManifest("application", compiler.info, native, {
        image: NAMES.NATIVE_IMAGE,
        preludeSha256: aot.preludeSha256,
      }),
      null,
      2,
    );
    const inputs: SkpkInput[] = [
      ...kept,
      { name: NAMES.NATIVE_MANIFEST, kind: KIND.NATIVE_MANIFEST, data: new TextEncoder().encode(manifest) },
      {
        name: NAMES.NATIVE_IMAGE,
        kind: KIND.NATIVE_IMAGE,
        data: new Uint8Array(deflateRawSync(native.image, { level: 9 })),
      },
    ];
    const exe = join(split, flavor, `aotbench-split-${flavor}.exe`);
    stamp({ host: host.exe, out: exe, payload: (skew) => writeSkpk(inputs, { skew }), log: () => {} });
    built.set(`split-${flavor}`, { exe, name, env: {}, imageBytes: native.image.length });
    log(`split-${flavor}: image ${(native.image.length / 2 ** 20).toFixed(1)} MiB, sk-aot ${native.ms} ms`);
  }
}

type Sample = Record<string, number>;
const samples = new Map<VariantName, Sample[]>(variants.map((v) => [v, []]));
const perfLine = (text: string, pattern: RegExp) => {
  const m = pattern.exec(text);
  return m ? Number(m[1]) : NaN;
};

async function once(variant: VariantName, run: number): Promise<Sample> {
  const b = built.get(variant)!;
  const t0 = performance.now();
  const app = await launch({
    app: `aotbench-${variant}`,
    exe: b.exe,
    packaged: b.name,
    label: `aotbench-${variant}-${run}`,
    window: [900, 700],
    env: { SK_PERF: "1", ...b.env },
  });
  try {
    await expect(app.locator(".ready")).toHaveCount(1);
    const visible = performance.now() - t0;
    const deadline = Date.now() + 180_000;
    let state: { mainAt: number; mountedAt: number; rounds: Sample[]; done: boolean; error: string | null } | null =
      null;
    while (Date.now() < deadline) {
      state = JSON.parse(String(await app.eval("JSON.stringify(globalThis.__aotbench)")));
      if (state?.done) break;
      await Bun.sleep(250);
    }
    if (!state?.done) throw Error(`${variant}: the bench did not finish`);
    if (state.error) throw Error(`${variant}: ${state.error}`);
    const hostLog = readFileSync(app.hostLog, "utf8");
    const engine = /selected application image/.test(hostLog) ? "aot" : "jit";
    if ((variant === "jit") !== (engine === "jit")) throw Error(`${variant} ran under ${engine}`);
    const sample: Sample = {
      "startup.spawnToMounted": visible,
      "startup.bootToFirstFrame": perfLine(hostLog, /boot->first-frame ([\d.]+)ms/),
      "startup.bootJs": perfLine(hostLog, /boot js ([\d.]+)ms/),
      "startup.mainToMounted": state.mountedAt - state.mainAt,
    };
    state.rounds.forEach((round, i) => {
      const tag = i === 0 ? "cold" : i === state!.rounds.length - 1 ? "warm" : null;
      if (tag) for (const [k, v] of Object.entries(round)) sample[`${tag}.${k}`] = v;
    });
    return sample;
  } finally {
    await app.close();
  }
}

for (let run = 0; run < runs; run++) {
  const order = run % 2 === 0 ? variants : [...variants].reverse();
  for (const variant of order) {
    const s = await once(variant, run);
    samples.get(variant)!.push(s);
    log(
      `run ${run + 1}/${runs} ${variant}: warm.total ${s["warm.total"]?.toFixed(0)} ms, cold.total ${s["cold.total"]?.toFixed(0)} ms`,
    );
  }
}

const quantile = (sorted: number[], q: number) => {
  const at = (sorted.length - 1) * q;
  const lo = Math.floor(at);
  return sorted[lo]! + (sorted[Math.min(lo + 1, sorted.length - 1)]! - sorted[lo]!) * (at - lo);
};
const metrics = [...new Set([...samples.values()].flatMap((list) => list.flatMap((s) => Object.keys(s))))];
interface Stat {
  median: number;
  p25: number;
  p75: number;
  min: number;
  max: number;
  n: number;
}
const summary: Record<string, Record<string, Stat>> = {};
for (const metric of metrics) {
  summary[metric] = {};
  for (const variant of variants) {
    const xs = samples
      .get(variant)!
      .map((s) => s[metric])
      .filter((x): x is number => Number.isFinite(x))
      .sort((a, b) => a - b);
    if (!xs.length) continue;
    summary[metric]![variant] = {
      median: quantile(xs, 0.5),
      p25: quantile(xs, 0.25),
      p75: quantile(xs, 0.75),
      min: xs[0]!,
      max: xs[xs.length - 1]!,
      n: xs.length,
    };
  }
}
const cell = (s: Stat | undefined) => (s ? `${s.median.toFixed(1)} [${s.p25.toFixed(0)}-${s.p75.toFixed(0)}]` : "-");
const header = ["metric, ms: median [p25-p75]", ...variants];
const lines = [header.join(" | "), header.map(() => "---").join(" | ")];
for (const metric of metrics) lines.push([metric, ...variants.map((v) => cell(summary[metric]![v]))].join(" | "));
console.log(`\n${lines.join("\n")}\n`);
const report = join(OUT, "results.json");
writeFileSync(
  report,
  JSON.stringify(
    {
      vue: vueVersion,
      host: host.exe,
      compiler: compiler.path,
      images: Object.fromEntries([...built].map(([k, b]) => [k, b.imageBytes])),
      samples: Object.fromEntries(samples),
      summary,
    },
    null,
    2,
  ) + "\n",
);
log(`samples: ${report}`);
