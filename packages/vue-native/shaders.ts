// vue-native effect shader compiler: `.slang` -> checked-in `.hlsl`.
//
// Usage: bun shaders.ts [--check] [--verbose]
//
//   (no flags)  regenerate crates/vn-effects/generated/*.hlsl + manifest.json
//   --check     regenerate into a temp dir and FAIL if the checked-in files
//               differ. This is the CI gate — `cargo build` never runs slangc,
//               so a stale generated file is otherwise invisible.
//
// WHY THE OUTPUT IS CHECKED IN (docs/SHADERS.md §2.3, owner decision 3):
//   * debug builds hot-reload HLSL with `D3DCompileFromFile` on a real path in
//     this repo — an effect author gets a sub-second edit->see loop and can
//     drop to raw HLSL to debug what Slang emitted;
//   * a Slang upgrade that silently changes codegen shows up as a PR diff;
//   * contributors and CI need no slangc and no second SDK to build or ship.
//
// AUTHORING MODEL (PEWPEW's `IUIMaterial` pattern, adopted verbatim):
// the author writes ONLY `struct <Name> : IEffect { float4 shade(...) }`.
// This script APPENDS an engine-owned wrapper after the user source — appended,
// never prepended, so Slang's diagnostics keep the author's line numbers — and
// that wrapper owns the register contract, the instance fetch, the rounded-rect
// coverage and the premultiplied clamp. An effect therefore cannot break the
// renderer's binding layout or hand DirectComposition an out-of-gamut pixel.
//
// The VERTEX stage is NOT generated here. It is hand-written HLSL living in
// `vendor/zed/crates/gpui_windows/src/effects.hlsl`, because gpui clips every
// primitive with `float4 : SV_ClipDistance` and no spelling of that survives
// both Slang's front end and fxc — re-probed at vs_5_0, see the header of
// effects.hlsl for the exact errors.

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

// ---------------------------------------------------------------------------
// layout
// ---------------------------------------------------------------------------

const REPO = resolve(import.meta.dir, "../..");
const SHADERS = join(REPO, "crates/vn-effects/shaders");
const MODULES = join(SHADERS, "modules");
const GENERATED = join(REPO, "crates/vn-effects/generated");

/// Bumped whenever the wrapper below changes in a way that alters codegen.
/// Rides in every generated header and in manifest.json, so a wrapper edit
/// busts every artifact even when the .slang sources are untouched — PEWPEW's
/// `SHADER_COMPILER_SCHEMA` trick.
const SCHEMA = 3;

/// Pinned by sha256 in `crates/vn-effects/tools/slang.manifest.json`; see
/// `fetch-slang.ts`. 2026.12 deliberately matches PEWPEW so there is one set
/// of codegen quirks to know across both projects.
const SLANG_VERSION = "2026.12";

function findSlangc(): string {
  const candidates = [
    process.env.VN_SLANGC_PATH,
    join(REPO, "tools/slang/win-x64/slangc.exe"),
    // PEWPEW's checkout, which pins the same version. Handy on this machine;
    // `bun fetch-slang.ts` populates tools/ for everyone else.
    "Z:/!argon/PEWPEW/tools/slang/win-x64/slangc.exe",
  ].filter(Boolean) as string[];
  for (const c of candidates) if (existsSync(c)) return c;
  throw new Error(
    "slangc not found. Run `bun fetch-slang.ts` (sha256-pinned download) or set VN_SLANGC_PATH.\n" +
      `tried:\n  ${candidates.join("\n  ")}`,
  );
}

// ---------------------------------------------------------------------------
// the engine-owned fragment wrapper
// ---------------------------------------------------------------------------

/// Keep in sync with:
///   * `gpui::EffectQuad`               (vendor/zed/crates/gpui/src/scene.rs)
///   * `struct EffectQuad` in effects.hlsl (the vertex half of the same buffer)
/// 32 words / 128 bytes.
function wrapper(structName: string): string {
  return `
// ===========================================================================
//  engine-generated wrapper — DO NOT EDIT, DO NOT COPY INTO AN EFFECT
//  (appended by packages/vue-native/shaders.ts, schema ${SCHEMA})
// ===========================================================================

cbuffer GlobalParams : register(b0)
{
    float4 vn_gamma_ratios;
    float2 vn_viewport_size;
    float vn_grayscale_enhanced_contrast;
    float vn_subpixel_enhanced_contrast;
    uint vn_is_bgr;
    uint3 vn_global_pad;
};

struct VnBounds { float2 origin; float2 size; };
struct VnCorners { float top_left; float top_right; float bottom_right; float bottom_left; };

struct VnEffectQuad
{
    uint order;
    uint effect_id;
    VnBounds bounds;
    VnBounds content_mask;
    VnCorners corner_radii;
    float4 params0;
    float4 params1;
    float opacity;
    float time;
    float bleed;
    float backdrop_blur_radius;
    float scale;
    uint flags;
    uint4 pad;
};

// t1 is the instance buffer for EVERY gpui pipeline; t0 is the processed
// backdrop (the Dual-Kawase pyramid's top level) and t2 the untouched
// render-target snapshot. s1 clamps — s0 wraps, which bleeds the opposite
// screen edge into anything sampling near the border.
StructuredBuffer<VnEffectQuad> vn_effect_quads : register(t1);
Texture2D<float4> vn_backdrop_input : register(t0);
Texture2D<float4> vn_backdrop_original : register(t2);
SamplerState vn_clamp_sampler : register(s1);

static const uint VN_FLAG_NEEDS_BACKDROP = 1u;

struct VnEffectFragmentInput
{
    nointerpolation uint id : TEXCOORD0;
    float4 position : SV_Position;
};

float vn_pick_corner_radius(float2 center_to_point, VnCorners radii)
{
    if (center_to_point.x < 0.0)
    {
        return center_to_point.y < 0.0 ? radii.top_left : radii.bottom_left;
    }
    return center_to_point.y < 0.0 ? radii.top_right : radii.bottom_right;
}

float vn_quad_sdf(float2 pt, VnBounds bounds, VnCorners radii)
{
    float2 half_size = bounds.size / 2.0;
    float2 center = bounds.origin + half_size;
    float2 center_to_point = pt - center;
    float radius = vn_pick_corner_radius(center_to_point, radii);
    float2 corner_to_point = abs(center_to_point) - half_size;
    float2 corner_center_to_point = corner_to_point + radius;
    if (radius == 0.0)
    {
        return max(corner_center_to_point.x, corner_center_to_point.y);
    }
    return length(max(float2(0.0, 0.0), corner_center_to_point))
        + min(0.0, max(corner_center_to_point.x, corner_center_to_point.y))
        - radius;
}

// The DirectComposition swap chain is premultiplied; keep rgb <= a so the
// compositor never sees an out-of-gamut pixel. Lives HERE and not in effect
// code precisely so an effect cannot skip it.
float4 vn_clamp_premultiplied(float4 color)
{
    color.a = saturate(color.a);
    color.rgb = clamp(color.rgb, 0.0, float3(color.a, color.a, color.a));
    return color;
}

[shader("fragment")]
float4 effect_fragment(VnEffectFragmentInput input) : SV_Target
{
    VnEffectQuad q = vn_effect_quads[input.id];
    float2 position = input.position.xy;
    // SV_Position is the pixel centre, so this lands on a texel centre of the
    // full-resolution snapshot.
    float2 uv = position / vn_viewport_size;

    // \`bounds\`/\`corner_radii\` describe the DRAW quad, which is the element's
    // border box dilated by \`bleed\` (an outer glow needs somewhere to land).
    // Dilating a rounded rect by b is a Minkowski sum with a disc of radius b,
    // so its SDF is exactly the element's minus b — recover one from the other.
    float sd_draw = vn_quad_sdf(position, q.bounds, q.corner_radii);
    float sd_element = sd_draw + q.bleed;

    float2 element_origin = q.bounds.origin + q.bleed;
    float2 element_size = max(q.bounds.size - 2.0 * q.bleed, float2(1.0, 1.0));

    EffectInput i;
    i.uv = uv;
    i.px = position - element_origin;
    i.size = element_size;
    i.local = i.px / element_size;
    i.backdrop = (q.flags & VN_FLAG_NEEDS_BACKDROP) != 0u
        ? vn_backdrop_input.SampleLevel(vn_clamp_sampler, uv, 0.0)
        : float4(0.0, 0.0, 0.0, 0.0);
    i.sd = sd_element;
    i.coverage = saturate(0.5 - sd_element);
    i.time = q.time;
    i.scale = q.scale;
    i.params0 = q.params0;
    i.params1 = q.params1;

    ${structName} vn_material;
    float4 shaded = vn_clamp_premultiplied(vn_material.shade(i));

    // Coverage of the DRAW quad, which is what keeps the primitive's own
    // antialiased boundary correct no matter what the effect returns.
    float cov = saturate(0.5 - sd_draw) * saturate(q.opacity);

    if ((q.flags & VN_FLAG_NEEDS_BACKDROP) != 0u)
    {
        // Backdrop effects run with blending DISABLED and own every pixel of
        // their quad, so they must restore the untouched snapshot outside the
        // coverage — the same trick blur_rect uses to keep its edge crisp.
        float4 original = vn_backdrop_original.SampleLevel(vn_clamp_sampler, uv, 0.0);
        return vn_clamp_premultiplied(lerp(original, shaded, cov));
    }
    return vn_clamp_premultiplied(shaded * cov);
}
`;
}

// ---------------------------------------------------------------------------
// source headers
// ---------------------------------------------------------------------------

type EffectSource = {
  file: string;
  name: string;
  structName: string;
  backdrop: boolean;
  source: string;
};

function parseHeader(file: string, source: string): EffectSource {
  const directive = (key: string): string | undefined => {
    const m = source.match(new RegExp(`^//!\\s*vn-${key}\\s*:\\s*(.+)$`, "m"));
    return m ? m[1].trim() : undefined;
  };
  const name = directive("effect");
  const structName = directive("struct");
  if (!name || !structName) {
    throw new Error(
      `${basename(file)}: missing \`//! vn-effect: <name>\` / \`//! vn-struct: <Struct>\` header`,
    );
  }
  return {
    file,
    name,
    structName,
    backdrop: directive("backdrop") === "true",
    source,
  };
}

function discover(): EffectSource[] {
  return readdirSync(SHADERS)
    .filter((f) => f.endsWith(".slang"))
    .sort()
    .map((f) => {
      const path = join(SHADERS, f);
      return parseHeader(path, readFileSync(path, "utf8"));
    });
}

// ---------------------------------------------------------------------------
// slangc
// ---------------------------------------------------------------------------

/// The exact command line, recorded verbatim in every generated header so a
/// regression can be reproduced without reading this file.
function slangArgs(input: string, output: string, reflection: string): string[] {
  return [
    "-target", "hlsl",
    "-profile", "sm_5_0",
    "-stage", "fragment",
    "-entry", "effect_fragment",
    // Pinned even though no effect takes a matrix yet: PEWPEW lost a day to
    // 2026.14 flipping this default under them.
    "-matrix-layout-row-major",
    "-I", MODULES,
    "-reflection-json", reflection,
    "-o", output,
    input,
  ];
}

type Diagnostic = { severity: string; file: string; line: number; col: number; message: string };

/// Slang emits codespan-ish text; turn it into structured records and subtract
/// the wrapper offset so a diagnostic in USER code keeps the author's line
/// numbers (which is the whole reason the wrapper is appended, not prepended).
function parseDiagnostics(text: string, userLines: number, file: string): Diagnostic[] {
  const out: Diagnostic[] = [];
  const re = /^(.*?)\((\d+)\):\s*(error|warning|note)\s+\S+:\s*(.*)$/gm;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) {
    const line = Number(m[2]);
    out.push({
      severity: m[3],
      file: line <= userLines ? file : "<engine wrapper>",
      line: line <= userLines ? line : line - userLines,
      col: 0,
      message: m[4],
    });
  }
  // The `--> file:line:col` form Slang uses for its richer diagnostics.
  const re2 = /^\s*-->\s*(.+?):(\d+):(\d+)/gm;
  while ((m = re2.exec(text))) {
    const line = Number(m[2]);
    if (out.some((d) => d.line === line)) continue;
    out.push({
      severity: "error",
      file: line <= userLines ? file : "<engine wrapper>",
      line: line <= userLines ? line : line - userLines,
      col: Number(m[3]),
      message: "",
    });
  }
  return out;
}

function sha256(text: string): string {
  return createHash("sha256").update(text).digest("hex");
}

/// The one thing worth keeping out of `-reflection-json`: the register each
/// top-level parameter landed on. That is the contract with the D3D11 renderer
/// (t0 processed backdrop, t1 instance buffer, t2 snapshot, b0 globals, s1
/// clamp), and a Slang upgrade that moved any of them would show up here as a
/// reviewable diff rather than as a black rectangle three weeks later.
function bindings(reflection: any): Array<Record<string, unknown>> {
  const params = Array.isArray(reflection?.parameters) ? reflection.parameters : [];
  return params.map((p: any) => ({
    name: p?.name,
    kind: p?.binding?.kind,
    index: p?.binding?.index,
  }));
}

type Compiled = {
  effect: EffectSource;
  hlsl: string;
  reflection: unknown;
  commandLine: string;
};

function compile(slangc: string, effect: EffectSource, workdir: string): Compiled {
  const userSource = effect.source;
  const userLines = userSource.split("\n").length;
  const combined = `${userSource}\n${wrapper(effect.structName)}`;

  // slangc CLI quirk (documented by PEWPEW, hit again here): it reads and
  // writes real files only — there is no stdin/stdout mode for HLSL output,
  // and it mangles non-ASCII paths. Everything therefore runs in an ASCII
  // temp dir, never in-place next to the sources.
  const input = join(workdir, `${effect.name}.slang`);
  const output = join(workdir, `${effect.name}.hlsl`);
  const reflection = join(workdir, `${effect.name}.reflection.json`);
  writeFileSync(input, combined);

  const args = slangArgs(input, output, reflection);
  const run = spawnSync(slangc, args, { encoding: "utf8" });
  const stderr = `${run.stdout ?? ""}${run.stderr ?? ""}`;

  if (run.status !== 0 || !existsSync(output)) {
    const diags = parseDiagnostics(stderr, userLines, effect.file);
    const rendered = diags.length
      ? diags
          .map((d) => `  ${d.file}:${d.line}${d.col ? `:${d.col}` : ""}: ${d.severity} ${d.message}`)
          .join("\n")
      : `  ${stderr.trim()}`;
    throw new Error(`slangc failed for ${basename(effect.file)}:\n${rendered}`);
  }
  if (stderr.trim()) {
    for (const d of parseDiagnostics(stderr, userLines, effect.file)) {
      console.warn(`  ${d.file}:${d.line}: ${d.severity} ${d.message}`);
    }
  }

  // The recorded command line uses stable, repo-relative placeholders — the
  // temp dir changes every run and would otherwise churn the diff.
  const commandLine = [
    "slangc",
    ...slangArgs(`shaders/${effect.name}.slang+wrapper`, `generated/${effect.name}.hlsl`, "-")
      .slice(0, -1)
      .map((a) => (a === MODULES ? "shaders/modules" : a)),
    `shaders/${effect.name}.slang+wrapper`,
  ].join(" ");

  // slangc CLI quirk #2: `#line` directives carry the ABSOLUTE path of the
  // input, so the temp directory's random name would land in the checked-in
  // artifact and churn the diff on every regeneration. Rewrite it to a stable
  // repo-relative name. (Line numbers are left alone — up to `userLines` they
  // are the author's; past that they are the appended wrapper's.)
  const stablePath = `crates/vn-effects/shaders/${effect.name}.slang+wrapper`;
  const body = readFileSync(output, "utf8")
    .split(input.replace(/\\/g, "/"))
    .join(stablePath)
    .split(input)
    .join(stablePath);
  const header =
    `// GENERATED by packages/vue-native/shaders.ts — DO NOT EDIT.\n` +
    `// Regenerate with \`bun shaders.ts\` (CI gate: \`bun shaders.ts --check\`).\n` +
    `//\n` +
    `//   effect:    ${effect.name}\n` +
    `//   source:    crates/vn-effects/shaders/${basename(effect.file)}\n` +
    `//   slangc:    ${SLANG_VERSION}\n` +
    `//   schema:    ${SCHEMA}\n` +
    `//   command:   ${commandLine}\n` +
    `//   src-sha256:  ${sha256(userSource)}\n` +
    `//\n` +
    `// Compiled at ps_5_0 (effects only — the core gpui shaders stay at ps_4_1).\n` +
    `// The VERTEX half lives in gpui_windows/src/effects.hlsl and is shared by\n` +
    `// every effect; Slang cannot author it (SV_ClipDistance, see that file).\n\n`;

  return {
    effect,
    hlsl: header + body,
    // The full `-reflection-json` dump is ~24 KB per effect, almost all of it
    // the AST of the prelude's helpers. Only the binding table is worth
    // checking in: it is what proves the register contract survived Slang.
    reflection: existsSync(reflection)
      ? bindings(JSON.parse(readFileSync(reflection, "utf8")))
      : null,
    commandLine,
  };
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

function main() {
  const check = process.argv.includes("--check");
  const slangc = findSlangc();
  const effects = discover();
  if (effects.length === 0) throw new Error(`no .slang sources under ${SHADERS}`);

  const workdir = mkdtempSync(join(tmpdir(), "vn-shaders-"));
  let compiled: Compiled[];
  try {
    compiled = effects.map((e) => compile(slangc, e, workdir));
  } finally {
    rmSync(workdir, { recursive: true, force: true });
  }

  const prelude = readFileSync(join(MODULES, "vn.slang"), "utf8");
  const manifest = {
    schema: SCHEMA,
    slangc: SLANG_VERSION,
    preludeSha256: sha256(prelude),
    effects: compiled.map((c) => ({
      name: c.effect.name,
      struct: c.effect.structName,
      backdrop: c.effect.backdrop,
      source: `shaders/${basename(c.effect.file)}`,
      generated: `generated/${c.effect.name}.hlsl`,
      sourceSha256: sha256(c.effect.source),
      generatedSha256: sha256(c.hlsl),
      command: c.commandLine,
      reflection: c.reflection,
    })),
  };
  const manifestText = `${JSON.stringify(manifest, null, 2)}\n`;

  mkdirSync(GENERATED, { recursive: true });
  const wanted: Array<[string, string]> = [
    ...compiled.map((c) => [join(GENERATED, `${c.effect.name}.hlsl`), c.hlsl] as [string, string]),
    [join(GENERATED, "manifest.json"), manifestText],
  ];

  if (check) {
    const stale = wanted.filter(([path, text]) => {
      if (!existsSync(path)) return true;
      return readFileSync(path, "utf8") !== text;
    });
    if (stale.length) {
      console.error("generated shader artifacts are stale — run `bun shaders.ts`:");
      for (const [path] of stale) console.error(`  ${path}`);
      process.exit(1);
    }
    console.log(`shaders: ${compiled.length} effect(s) up to date`);
    return;
  }

  for (const [path, text] of wanted) writeFileSync(path, text);
  console.log(
    `shaders: wrote ${compiled.length} effect(s) -> ${GENERATED}\n  ` +
      compiled.map((c) => c.effect.name).join(", "),
  );
}

main();
