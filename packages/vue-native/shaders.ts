// vue-native effect shader compiler: `.slang` -> `.hlsl` + a reflected schema.
//
// Usage
//   bun shaders.ts                       regenerate the BUILT-IN effects into
//                                        crates/vn-effects/generated/
//   bun shaders.ts --check               CI gate: regenerate into a temp dir and
//                                        fail if the checked-in files differ
//   bun shaders.ts --app <dir> --out <dir> [--watch]
//                                        compile an APP's shaders/*.slang into
//                                        <out>/<name>.hlsl + <out>/effects.json,
//                                        which the host loads via VN_EFFECTS_DIR
//
// This script is invocable standalone and is deliberately NOT wired into
// build.ts: a `.slang` edit is a different cadence from a `.vue` edit, an app
// with no shaders/ dir must not pay for slangc, and a contributor without
// slangc must still be able to build and ship.
//
// ---------------------------------------------------------------------------
// THE AUTHORING CONTRACT, AND WHY IT IS REFLECTION-DRIVEN
// ---------------------------------------------------------------------------
//
// An effect author writes ONE thing:
//
//     import vn;
//
//     [VnEffect("ripple")]
//     struct Ripple : IEffect
//     {
//         [VnDefault("1.5")] [VnRange(0.0, 8.0)] float speed;
//         [VnColor] [VnDefault("#7fd4ff")]       float4 tint;
//         float4 shade(EffectInput input) { ... }
//     };
//
// slangc reflects the struct's user-defined attributes, its field ORDER and its
// field TYPES (`-reflection-json`). That reflection is the single source of
// truth for:
//
//   * the CSS function name                    <- [VnEffect("ripple")]
//   * the argument order and the packing slots <- field declaration order
//   * each argument's CSS type                 <- field type + [VnLength]/[VnColor]
//   * defaults, ranges, optionality            <- [VnDefault]/[VnRange]/[VnOptional]
//   * whether the effect needs the backdrop,
//     dilates its quad, or animates            <- [VnBackdrop]/[VnBleed]/…
//
// so `--shading: ripple(2.0 #ff8800)` parses with ZERO manual registration on
// the Rust or JS side. The schema is emitted as `effects.json` next to the
// HLSL; `crates/vn-effects` reads it and `crates/vn-css` validates against it.
//
// The engine's half of the shader is NOT string-templated. It lives in the
// `vn` module as `vn_begin()` + `vn_finish<T : IEffect>()`, and Slang
// specialises the generic on the author's type at compile time. The wrapper
// this script appends is therefore only the parameter unpacking — the one part
// that genuinely cannot be written without knowing the field names, which is
// exactly what reflection supplies.
//
// The VERTEX stage is not generated here at all: it is hand-written HLSL in
// `vendor/zed/crates/gpui_windows/src/effects.hlsl`, because gpui clips every
// primitive with `float4 : SV_ClipDistance` and no spelling of that survives
// both Slang's front end and fxc at any profile. See that file for the errors.
//
// ---------------------------------------------------------------------------
// WHY THE BUILT-IN OUTPUT IS CHECKED IN (docs/SHADERS.md §2.3)
// ---------------------------------------------------------------------------
//   * debug builds hot-reload HLSL with `D3DCompileFromFile` on a real path in
//     this repo, so an effect author gets a sub-second edit->see loop and can
//     drop to raw HLSL to debug what Slang emitted;
//   * a Slang upgrade that silently changes codegen shows up as a PR diff;
//   * contributors and CI need no slangc and no second SDK to build or ship.
//
// App-supplied effects invert exactly one of those: their HLSL is compiled by
// D3DCompile at APP STARTUP (both debug and release), because the app is not
// part of the Rust build. See docs/SHADERS-AUTHORING.md.

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  watch,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

// ---------------------------------------------------------------------------
// layout
// ---------------------------------------------------------------------------

const REPO = resolve(import.meta.dir, "../..");
const BUILTIN_SHADERS = join(REPO, "crates/vn-effects/shaders");
const MODULES = join(BUILTIN_SHADERS, "modules");
const GENERATED = join(REPO, "crates/vn-effects/generated");

/// Bumped whenever the prelude or the generated wrapper changes in a way that
/// alters codegen or the schema shape. Rides in every generated header and in
/// every effects.json, so a wrapper edit busts every artifact even when the
/// `.slang` sources are untouched — PEWPEW's `SHADER_COMPILER_SCHEMA` trick.
/// Rust refuses to load an effects.json whose schema it does not know.
/// 5: `VnEffectQuad` carries the content mask's corner radii where its padding was.
const SCHEMA = 5;

/// Pinned by sha256 in `crates/vn-effects/tools/slang.manifest.json`; see
/// `fetch-slang.ts`. 2026.12 deliberately matches PEWPEW so there is one set of
/// codegen quirks to know across both projects.
const SLANG_VERSION = "2026.12";

/// The built-in numbering is a FOUR-way contract: this list, `gpui::effect_id`,
/// the pipeline table in `gpui_windows`, and the ids in
/// `crates/vn-effects/generated/effects.json`. Alphabetical file order is NOT
/// that order (frost, glow, noise), so it is written down here.
const BUILTIN_ORDER = ["frost", "noise", "glow"];

/// Eight floats per instance — `EffectQuad::params0` + `params1`. Raising this
/// means widening the primitive, which costs bytes on every effect quad.
const PARAM_SLOTS = 8;

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
// the schema, as it lands in effects.json
// ---------------------------------------------------------------------------

type ParamKind = "number" | "length" | "color";

type ParamSchema = {
  name: string;
  kind: ParamKind;
  /// First of `kind === "color" ? 4 : 1` consecutive parameter slots.
  slot: number;
  required: boolean;
  /// Four floats; only the first is meaningful unless `kind === "color"`.
  default: [number, number, number, number];
  /// Inclusive clamp for a scalar, or null.
  range: [number, number] | null;
};

type EffectSchema = {
  name: string;
  struct: string;
  doc: string | null;
  backdrop: boolean;
  params: ParamSchema[];
  backdropRadiusParam: number | null;
  bleedParam: number | null;
  animatedParam: number | null;
  signature: string;
};

// ---------------------------------------------------------------------------
// the engine-owned wrapper
// ---------------------------------------------------------------------------
//
// Two forms. The PROBE wrapper exists only to make the author's struct a
// reflectable shader parameter; it never reads a field, so Slang strips the
// `cbuffer` from the emitted HLSL while `-reflection-json` still describes it
// in full (measured with slangc 2026.12). That is what lets the schema be
// discovered without the author declaring anything twice.
//
// The REAL wrapper unpacks the parameter slots into the author's named fields —
// the one thing that cannot be written without knowing those names — and hands
// the struct to the engine's generic composition.

const WRAPPER_BANNER = `
// ===========================================================================
//  engine-generated wrapper — DO NOT EDIT, DO NOT COPY INTO AN EFFECT
//  (appended by packages/vue-native/shaders.ts, schema ${SCHEMA})
//  Appended, never prepended, so slangc diagnostics keep YOUR line numbers.
// ===========================================================================
`;

function probeWrapper(structName: string): string {
  return `${WRAPPER_BANNER}
// b3 is unused by gpui (b0 = globals, b1 = batch params). This declaration is
// never read, so it does not reach the HLSL; it exists solely so that slangc's
// reflection describes \`${structName}\` and its user-defined attributes.
ConstantBuffer<${structName}> vn_reflect : register(b3);

[shader("fragment")]
float4 vn_reflect_probe(float4 vn_position : SV_Position) : SV_Target
{
    return float4(0.0, 0.0, 0.0, 0.0);
}
`;
}

/// `params0` is slots 0..3, `params1` slots 4..7.
function slotExpr(slot: number): string {
  const bank = slot < 4 ? "params0" : "params1";
  return `vn_in.${bank}.${"xyzw"[slot % 4]}`;
}

function loadExpr(param: ParamSchema, components: number): string {
  if (components === 1) return slotExpr(param.slot);
  const parts: string[] = [];
  for (let c = 0; c < components; c += 1) parts.push(slotExpr(param.slot + c));
  return `float${components}(${parts.join(", ")})`;
}

function realWrapper(effect: EffectSchema, fields: Field[]): string {
  const assignments = fields
    .map((field) => {
      const param = effect.params[field.firstParam];
      return `    vn_material.${field.name} = ${loadExpr(param, field.components)};`;
    })
    .join("\n");
  return `${WRAPPER_BANNER}
// Kept for the record: reflection reads this declaration, the HLSL backend
// strips it (nothing references it), and the renderer never binds b3.
ConstantBuffer<${effect.struct}> vn_reflect : register(b3);

[shader("fragment")]
float4 effect_fragment(VnFragmentInput vn_frag) : SV_Target
{
    EffectInput vn_in = vn_begin(vn_frag);

    // Parameter unpacking, generated from slangc's reflection of
    // \`${effect.struct}\`: field order IS slot order IS CSS argument order.
    ${effect.struct} vn_material;
${assignments || "    // (this effect declares no parameters)"}

    return vn_finish(vn_material, vn_in);
}
`;
}

// ---------------------------------------------------------------------------
// slangc
// ---------------------------------------------------------------------------

type Diagnostic = { severity: string; file: string; line: number; col: number; message: string };

/// Slang emits codespan-ish text; turn it into structured records and subtract
/// the wrapper offset so a diagnostic in USER code keeps the author's line
/// numbers (which is the whole reason the wrapper is appended, not prepended).
function parseDiagnostics(text: string, userLines: number, file: string): Diagnostic[] {
  const out: Diagnostic[] = [];
  const seen = new Set<string>();
  const push = (severity: string, line: number, col: number, message: string) => {
    const inUser = line <= userLines;
    const key = `${severity}:${line}:${col}:${message}`;
    if (seen.has(key)) return;
    seen.add(key);
    out.push({
      severity,
      file: inUser ? file : "<engine wrapper>",
      line: inUser ? line : line - userLines,
      col,
      message,
    });
  };

  // Slang's codespan form. The headline carries the message, the `-->` line
  // carries the location, and the caret block below it repeats the message with
  // more detail — which is the one worth showing, so prefer it when present:
  //
  //   error[E30015]: undefined identifier
  //     --> …/ripple.slang:10:36
  //      |
  //   10 |     return float4(input.local, nope, amount);
  //      |                                ^^^^ undefined identifier 'nope'.
  const codespan =
    /^(error|warning|note)(?:\[[^\]]*\])?:[ \t]*(.*)\r?\n[ \t]*-->[ \t]*(.+?):(\d+):(\d+)([\s\S]*?)(?=^(?:error|warning|note)\b|^-{2,}'|\Z)/gm;
  let m: RegExpExecArray | null;
  while ((m = codespan.exec(text))) {
    const detail = /\^+[ \t]+(.+)$/m.exec(m[6] ?? "");
    push(m[1], Number(m[4]), Number(m[5]), (detail ? detail[1] : m[2]).trim());
  }

  // The `file(line): severity CODE: message` form slangc uses for some
  // diagnostics (and fxc uses for all of them).
  const flat = /^(.*?)\((\d+)(?:,(\d+))?\):\s*(error|warning|note)\s+\S+:\s*(.*)$/gm;
  while ((m = flat.exec(text))) {
    push(m[4], Number(m[2]), Number(m[3] ?? 0), m[5]);
  }
  return out;
}

function renderDiagnostics(diags: Diagnostic[], fallback: string): string {
  if (!diags.length) return `  ${fallback.trim()}`;
  return diags
    .map((d) => `  ${d.file}:${d.line}${d.col ? `:${d.col}` : ""}: ${d.severity} ${d.message}`)
    .join("\n");
}

function sha256(text: string): string {
  return createHash("sha256").update(text).digest("hex");
}

type RunResult = { hlsl: string | null; reflection: any; stderr: string; ok: boolean };

/// slangc CLI bug (2026.12, hit here): `-reflection-json` re-emits the source
/// spelling of a string literal instead of JSON-escaping its VALUE. An
/// apostrophe therefore comes out as `\'` —
///
///     "arguments": [ "soft halo centred on the element\'s rounded edge" ]
///
/// — which is a C escape, not a JSON one, and `JSON.parse` rejects the whole
/// file. `[VnDoc("… element's …")]` in `glow.slang` reproduces it, and is left
/// there on purpose so this path stays exercised.
///
/// The repair is total for the escapes a Slang string literal can contain: turn
/// the C-only ones into their character, then double any backslash that is
/// still not a legal JSON escape. Failing loudly matters — returning `null`
/// here would surface much later as "your effect has no parameters".
function readReflection(path: string): any {
  if (!existsSync(path)) return null;
  const text = readFileSync(path, "utf8");
  try {
    return JSON.parse(text);
  } catch (first) {
    const repaired = text.replace(/"(?:[^"\\\n]|\\.)*"/g, (match) =>
      match
        .replace(/\\'/g, "'")
        .replace(/\\(?!["\\/bfnrtu])/g, "\\\\"),
    );
    try {
      return JSON.parse(repaired);
    } catch {
      throw new Error(
        `slangc wrote unparseable reflection JSON to ${path}: ${String(first)}\n` +
          "This is a slangc CLI bug, not a shader error. Check your attribute strings " +
          "for backslashes or quotes.",
      );
    }
  }
}

function runSlangc(
  slangc: string,
  includes: string[],
  combined: string,
  entry: string,
  workdir: string,
  stem: string,
): RunResult {
  // slangc CLI quirk (documented by PEWPEW, hit again here): it reads and
  // writes REAL FILES only — there is no stdin/stdout mode for HLSL output —
  // and it mangles non-ASCII paths. Everything therefore runs in an ASCII temp
  // dir, never in place next to the sources.
  const input = join(workdir, `${stem}.slang`);
  const output = join(workdir, `${stem}.hlsl`);
  const reflectionPath = join(workdir, `${stem}.reflection.json`);
  writeFileSync(input, combined);
  for (const stale of [output, reflectionPath]) {
    if (existsSync(stale)) rmSync(stale, { force: true });
  }

  const args = [
    "-target", "hlsl",
    "-profile", "sm_5_0",
    "-stage", "fragment",
    "-entry", entry,
    // Pinned even though no effect takes a matrix yet: PEWPEW lost a day to
    // 2026.14 flipping this default under them.
    "-matrix-layout-row-major",
    ...includes.flatMap((dir) => ["-I", dir]),
    "-reflection-json", reflectionPath,
    "-o", output,
    input,
  ];
  const run = spawnSync(slangc, args, { encoding: "utf8" });
  const stderr = `${run.stdout ?? ""}${run.stderr ?? ""}`;
  const ok = run.status === 0 && existsSync(output);
  return {
    ok,
    stderr,
    hlsl: ok ? readFileSync(output, "utf8") : null,
    reflection: readReflection(reflectionPath),
    // `input` is needed by the caller to rewrite #line paths.
    ...({ input } as any),
  };
}

/// The one thing worth keeping out of `-reflection-json`: the register each
/// top-level parameter landed on. That is the contract with the D3D11 renderer
/// (t0 processed backdrop, t1 instance buffer, t2 snapshot, b0 globals, s1
/// clamp), and a Slang upgrade that moved any of them shows up here as a
/// reviewable diff rather than as a black rectangle three weeks later.
function bindings(reflection: any): Array<Record<string, unknown>> {
  const params = Array.isArray(reflection?.parameters) ? reflection.parameters : [];
  return params
    .filter((p: any) => p?.name !== "vn_reflect")
    .map((p: any) => ({ name: p?.name, kind: p?.binding?.kind, index: p?.binding?.index }));
}

// ---------------------------------------------------------------------------
// reflection -> schema
// ---------------------------------------------------------------------------

type Attribute = { name: string; arguments: unknown[] };
type Field = {
  name: string;
  components: number;
  attributes: Attribute[];
  /// Index into `EffectSchema.params` of this field's FIRST css argument.
  firstParam: number;
};

class SchemaError extends Error {}

function attr(attrs: Attribute[], name: string): Attribute | undefined {
  return attrs.find((a) => a.name === name);
}

/// `#7fd4ff`, `#7fd4ff80`, `#abc`, `1.5`, `0.2 0.6 1`, `0.2, 0.6, 1`.
function parseDefault(text: string, components: number, where: string): [number, number, number, number] {
  const out: [number, number, number, number] = [0, 0, 0, 0];
  const raw = text.trim();
  if (raw.startsWith("#")) {
    const hex = raw.slice(1);
    const wide = hex.length > 4;
    const step = wide ? 2 : 1;
    const groups = hex.length / step;
    if (![3, 4].includes(groups) || !/^[0-9a-fA-F]+$/.test(hex)) {
      throw new SchemaError(`${where}: \`${raw}\` is not a #rgb/#rgba/#rrggbb/#rrggbbaa colour`);
    }
    for (let c = 0; c < 4; c += 1) {
      if (c >= groups) {
        out[c] = 1;
        continue;
      }
      const piece = hex.slice(c * step, c * step + step);
      const value = parseInt(wide ? piece : piece + piece, 16);
      out[c] = value / 255;
    }
    return out;
  }
  const numbers = raw
    .split(/[\s,]+/)
    .filter(Boolean)
    .map((n) => {
      const v = Number(n);
      if (!Number.isFinite(v)) throw new SchemaError(`${where}: \`${n}\` is not a number`);
      return v;
    });
  if (numbers.length === 0) throw new SchemaError(`${where}: empty [VnDefault]`);
  if (numbers.length === 1) {
    // One value broadcasts, which is what `[VnDefault("0")]` on a float2 means.
    for (let c = 0; c < components; c += 1) out[c] = numbers[0];
    return out;
  }
  if (numbers.length !== components) {
    throw new SchemaError(
      `${where}: [VnDefault] has ${numbers.length} values but the field has ${components} component(s)`,
    );
  }
  for (let c = 0; c < components; c += 1) out[c] = numbers[c];
  return out;
}

/// Whether a `components`-wide field may start at `slot`: free, in range, and
/// not straddling a 16-byte (four-slot) boundary.
///
/// The no-straddle rule is HLSL's own. Scalars go anywhere, a `float4` is
/// therefore always four-aligned, and a `float2`/`float3` is bumped forward
/// only when it would straddle — which keeps `params0`/`params1` addressable by
/// swizzle in the generated wrapper.
function fits(used: boolean[], slot: number, components: number): boolean {
  if (slot < 0 || slot + components > PARAM_SLOTS) return false;
  if (components > 1 && Math.floor(slot / 4) !== Math.floor((slot + components - 1) / 4)) {
    return false;
  }
  for (let c = 0; c < components; c += 1) if (used[slot + c]) return false;
  return true;
}

/// The first slot a `components`-wide field can occupy, or -1.
function allocate(used: boolean[], components: number): number {
  for (let slot = 0; slot + components <= PARAM_SLOTS; slot += 1) {
    if (fits(used, slot, components)) return slot;
  }
  return -1;
}

function fieldComponents(type: any, where: string): number {
  if (type?.kind === "scalar") {
    if (type.scalarType !== "float32") {
      throw new SchemaError(
        `${where}: parameters are floats in the instance buffer, so \`${type.scalarType}\` ` +
          "is not a legal parameter type — declare it `float` and compare in `shade`",
      );
    }
    return 1;
  }
  if (type?.kind === "vector") {
    if (type.elementType?.scalarType !== "float32") {
      throw new SchemaError(`${where}: only float vectors may be parameters`);
    }
    const n = Number(type.elementCount);
    if (![2, 3, 4].includes(n)) throw new SchemaError(`${where}: unsupported vector width ${n}`);
    return n;
  }
  throw new SchemaError(
    `${where}: unsupported parameter type \`${type?.kind ?? "?"}\` — a parameter must be ` +
      "`float`, `float2`, `float3` or `float4` (matrices, arrays, textures and " +
      "samplers cannot ride in the 8-float parameter block)",
  );
}

const COMPONENT_SUFFIX = ["x", "y", "z", "w"];

function buildSchema(reflection: any, structName: string, source: string): {
  effect: EffectSchema;
  fields: Field[];
} {
  const param = (reflection?.parameters ?? []).find((p: any) => p?.name === "vn_reflect");
  const element = param?.type?.elementType;
  if (!element || element.kind !== "struct") {
    throw new SchemaError(
      `${source}: slangc reflected no struct for \`${structName}\` — this is an internal ` +
        "error in shaders.ts, not in your shader",
    );
  }

  const structAttrs: Attribute[] = element.userAttribs ?? [];
  const effectAttr = attr(structAttrs, "VnEffect");
  if (!effectAttr) {
    throw new SchemaError(
      `${source}: \`struct ${structName}\` has no [VnEffect("<name>")] attribute — that ` +
        "attribute is what gives the effect its CSS function name",
    );
  }
  const name = String(effectAttr.arguments?.[0] ?? "");
  if (!/^[a-z][a-z0-9-]*$/.test(name)) {
    throw new SchemaError(
      `${source}: [VnEffect("${name}")] — an effect name must be a lowercase CSS ident ` +
        "(`[a-z][a-z0-9-]*`), because it becomes a CSS function name",
    );
  }
  const docAttr = attr(structAttrs, "VnDoc");

  const params: ParamSchema[] = [];
  const fields: Field[] = [];
  let backdropRadiusParam: number | null = null;
  let bleedParam: number | null = null;
  let animatedParam: number | null = null;
  let backdrop = attr(structAttrs, "VnBackdrop") !== undefined;
  // Which of the 8 float slots are already spoken for. Fields are packed in
  // declaration order into the first slot that fits, so a `[VnSlot]`-pinned
  // field simply reserves its run first and the rest flow around it.
  const used: boolean[] = new Array(PARAM_SLOTS).fill(false);
  // Reserve every `[VnSlot]`-pinned run up front, so pinning does not depend on
  // where the field happens to sit in the declaration order.
  const pinned = new Map<string, number>();
  for (const field of element.fields ?? []) {
    const slotAttr = attr(field.userAttribs ?? [], "VnSlot");
    if (!slotAttr) continue;
    const where = `${source}: \`${structName}.${String(field.name)}\``;
    const components = fieldComponents(field.type, where);
    const slot = Number(slotAttr.arguments?.[0]);
    if (!Number.isInteger(slot) || !fits(used, slot, components)) {
      throw new SchemaError(
        `${where}: [VnSlot(${slotAttr.arguments?.[0]})] is not a legal home for a ` +
          `${components}-component parameter — it must be free, inside 0..${PARAM_SLOTS - 1}, ` +
          "and must not straddle a four-slot boundary",
      );
    }
    for (let c = 0; c < components; c += 1) used[slot + c] = true;
    pinned.set(String(field.name), slot);
  }

  for (const field of element.fields ?? []) {
    const fieldName = String(field.name);
    const where = `${source}: \`${structName}.${fieldName}\``;
    const attrs: Attribute[] = field.userAttribs ?? [];
    const components = fieldComponents(field.type, where);

    const isColor = attr(attrs, "VnColor") !== undefined;
    const isLength = attr(attrs, "VnLength") !== undefined;
    const optional = attr(attrs, "VnOptional") !== undefined;
    const rangeAttr = attr(attrs, "VnRange");
    const defaultAttr = attr(attrs, "VnDefault");

    if (isColor && components !== 4) {
      throw new SchemaError(`${where}: [VnColor] is only legal on a float4 (RGBA)`);
    }
    if (isColor && isLength) {
      throw new SchemaError(`${where}: [VnColor] and [VnLength] are mutually exclusive`);
    }
    if (optional && components > 1 && !isColor) {
      // A bare vector expands to one CSS argument per component, and the
      // matcher decides whether to skip an optional parameter one argument at a
      // time — so half a `float2` could be filled in. Refuse rather than invent
      // an all-or-nothing rule the CSS grammar cannot express.
      throw new SchemaError(
        `${where}: [VnOptional] is not allowed on a float${components} — it expands to ` +
          `${components} CSS arguments and only some of them could be omitted. Split it into ` +
          "scalar fields, or drop [VnOptional].",
      );
    }

    let slot = pinned.get(fieldName) ?? -1;
    if (slot < 0) {
      slot = allocate(used, components);
      if (slot < 0) {
        throw new SchemaError(
          `${where}: no room left in the parameter block. An effect quad carries ` +
            `${PARAM_SLOTS} floats (EffectQuad::params0 + params1) and a <color> costs 4; ` +
            "drop or merge a parameter.",
        );
      }
      for (let c = 0; c < components; c += 1) used[slot + c] = true;
    }

    const defaults = defaultAttr
      ? parseDefault(String(defaultAttr.arguments?.[0] ?? ""), components, where)
      : ([0, 0, 0, 0] as [number, number, number, number]);
    const range = rangeAttr
      ? ([Number(rangeAttr.arguments?.[0]), Number(rangeAttr.arguments?.[1])] as [number, number])
      : null;

    const firstParam = params.length;
    if (isColor) {
      params.push({ name: fieldName, kind: "color", slot, required: !optional, default: defaults, range: null });
    } else {
      // A vector without [VnColor] expands to one CSS argument per component,
      // named `field.x`, `field.y`, … — the CSS grammar takes one value per
      // comma-separated argument, and inventing a bracketed sub-syntax for a
      // float2 would be a divergence with no browser precedent to copy.
      for (let c = 0; c < components; c += 1) {
        params.push({
          name: components === 1 ? fieldName : `${fieldName}.${COMPONENT_SUFFIX[c]}`,
          kind: isLength ? "length" : "number",
          slot: slot + c,
          required: !optional,
          default: [defaults[c], 0, 0, 0],
          range,
        });
      }
    }
    fields.push({ name: fieldName, components, attributes: attrs, firstParam });

    const marker = (attrName: string, current: number | null, requires: string) => {
      if (attr(attrs, attrName) === undefined) return current;
      if (current !== null) {
        throw new SchemaError(`${where}: [${attrName}] appears more than once in \`${structName}\``);
      }
      if (components !== 1) {
        throw new SchemaError(`${where}: [${attrName}] is only legal on a scalar \`float\``);
      }
      if (requires === "length" && !isLength) {
        throw new SchemaError(`${where}: [${attrName}] requires [VnLength] on the same field`);
      }
      return slot;
    };
    backdropRadiusParam = marker("VnBackdropRadius", backdropRadiusParam, "length");
    bleedParam = marker("VnBleed", bleedParam, "length");
    animatedParam = marker("VnAnimated", animatedParam, "number");
  }

  if (backdropRadiusParam !== null) backdrop = true;

  // Type-directed matching only works when a skippable parameter's kind differs
  // from the kind of whatever follows it — otherwise an argument is ambiguous
  // and `ripple(1)` could mean two different things.
  for (let i = 0; i < params.length; i += 1) {
    if (params[i].required) continue;
    const nextParam = params[i + 1];
    if (nextParam && nextParam.kind === params[i].kind) {
      throw new SchemaError(
        `${source}: \`${name}\` — optional parameter \`${params[i].name}\` has the same CSS ` +
          `type (${params[i].kind}) as the parameter after it (\`${nextParam.name}\`), so an ` +
          "argument in that position would be ambiguous. Make it required, reorder, or change " +
          "its type.",
      );
    }
  }

  // `<length:radius>`, `<number:strength>`, `<color:tint>` — but just `<color>`
  // when the parameter is itself called `color`, which reads better in the
  // advisory an author actually sees.
  const signature = `${name}(${params
    .map((p) => `<${p.name === p.kind ? p.kind : `${p.kind}:${p.name}`}>${p.required ? "" : "?"}`)
    .join(", ")})`;

  return {
    effect: {
      name,
      struct: structName,
      doc: docAttr ? String(docAttr.arguments?.[0] ?? "") : null,
      backdrop,
      params,
      backdropRadiusParam,
      bleedParam,
      animatedParam,
      signature,
    },
    fields,
  };
}

// ---------------------------------------------------------------------------
// source discovery
// ---------------------------------------------------------------------------

type EffectSource = { file: string; struct: string; source: string };

/// The struct name is the ONE thing reflection cannot tell us before we have
/// compiled, because the probe wrapper has to name the type. A file therefore
/// declares exactly one `struct <Name> : IEffect`.
function findStruct(file: string, source: string): string {
  const stripped = source.replace(/\/\/[^\n]*/g, "").replace(/\/\*[\s\S]*?\*\//g, "");
  const matches = [...stripped.matchAll(/\bstruct\s+([A-Za-z_]\w*)\s*:\s*(?:vn\.)?IEffect\b/g)];
  if (matches.length === 0) {
    throw new SchemaError(
      `${basename(file)}: no \`struct <Name> : IEffect\` found. An effect is a struct that ` +
        "conforms to the `IEffect` interface from the `vn` prelude (`import vn;`).",
    );
  }
  if (matches.length > 1) {
    throw new SchemaError(
      `${basename(file)}: ${matches.length} effect structs in one file ` +
        `(${matches.map((m) => m[1]).join(", ")}) — one effect per file`,
    );
  }
  return matches[0][1];
}

function discover(dir: string): EffectSource[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".slang"))
    .sort()
    .map((f) => {
      const path = join(dir, f);
      const source = readFileSync(path, "utf8");
      return { file: path, struct: findStruct(path, source), source };
    });
}

// ---------------------------------------------------------------------------
// compile one effect (two slangc passes)
// ---------------------------------------------------------------------------

type Compiled = {
  file: string;
  effect: EffectSchema;
  hlsl: string;
  reflection: unknown;
  sourceSha256: string;
};

/// `#line` directives carry the ABSOLUTE path of the staged input, so the temp
/// directory's random name would land in the artifact and churn the diff on
/// every regeneration. Rewrite it to a stable name. Line numbers are left
/// alone — up to `userLines` they are the author's, past that the wrapper's.
///
/// For an APP effect the stable name is the author's real `.slang` path, which
/// is what makes a runtime D3DCompile diagnostic point at their source.
function rewriteLineDirectives(hlsl: string, staged: string, stable: string): string {
  return hlsl.split(staged.replace(/\\/g, "/")).join(stable).split(staged).join(stable);
}

function compile(
  slangc: string,
  effectSource: EffectSource,
  workdir: string,
  includes: string[],
  stablePath: string,
): Compiled {
  const userSource = effectSource.source;
  const userLines = userSource.split("\n").length;
  const stem = basename(effectSource.file).replace(/\.slang$/, "");

  // PASS 1 — reflection only. The probe wrapper never touches the struct, so
  // its `cbuffer` is dead and Slang drops it from the HLSL we throw away; the
  // reflection JSON still describes the type in full.
  const probe = runSlangc(
    slangc,
    includes,
    `${userSource}\n${probeWrapper(effectSource.struct)}`,
    "vn_reflect_probe",
    workdir,
    `${stem}.probe`,
  );
  if (!probe.ok) {
    throw new Error(
      `slangc failed for ${basename(effectSource.file)}:\n` +
        renderDiagnostics(parseDiagnostics(probe.stderr, userLines, effectSource.file), probe.stderr),
    );
  }
  const { effect, fields } = buildSchema(probe.reflection, effectSource.struct, basename(effectSource.file));

  // PASS 2 — the real fragment shader, with the parameter unpacking generated
  // from what pass 1 learned.
  const wrapper = realWrapper(effect, fields);
  const real = runSlangc(
    slangc,
    includes,
    `${userSource}\n${wrapper}`,
    "effect_fragment",
    workdir,
    stem,
  );
  if (!real.ok) {
    throw new Error(
      `slangc failed for ${basename(effectSource.file)}:\n` +
        renderDiagnostics(parseDiagnostics(real.stderr, userLines, effectSource.file), real.stderr),
    );
  }
  for (const d of parseDiagnostics(real.stderr, userLines, effectSource.file)) {
    if (d.message) console.warn(`  ${d.file}:${d.line}: ${d.severity} ${d.message}`);
  }

  const staged = (real as any).input as string;
  const body = rewriteLineDirectives(real.hlsl!, staged, stablePath);
  const header =
    `// GENERATED by packages/vue-native/shaders.ts — DO NOT EDIT.\n` +
    `//\n` +
    `//   effect:     ${effect.name}\n` +
    `//   struct:     ${effect.struct}\n` +
    `//   source:     ${stablePath}\n` +
    `//   signature:  ${effect.signature}\n` +
    `//   slangc:     ${SLANG_VERSION}\n` +
    `//   schema:     ${SCHEMA}\n` +
    `//   src-sha256: ${sha256(userSource)}\n` +
    `//\n` +
    `// Compiled at ps_5_0 (effects only — the core gpui shaders stay at ps_4_1).\n` +
    `// The VERTEX half lives in gpui_windows/src/effects.hlsl and is shared by\n` +
    `// every effect; Slang cannot author it (SV_ClipDistance, see that file).\n\n`;

  return {
    file: effectSource.file,
    effect,
    hlsl: header + body,
    reflection: bindings(real.reflection),
    sourceSha256: sha256(userSource),
  };
}

function compileAll(
  slangc: string,
  sources: EffectSource[],
  includes: string[],
  stableFor: (s: EffectSource) => string,
): Compiled[] {
  // PEWPEW reports output-file races when several slangc processes run
  // concurrently; compiling serially avoids it and costs nothing at this scale.
  const workdir = mkdtempSync(join(tmpdir(), "vn-shaders-"));
  try {
    return sources.map((s) => compile(slangc, s, workdir, includes, stableFor(s)));
  } finally {
    rmSync(workdir, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------------------
// effects.json
// ---------------------------------------------------------------------------

function manifestText(
  kind: "builtin" | "user",
  compiled: Compiled[],
  preludeSha: string,
  // Built-ins record a REPO-RELATIVE source path: the file is checked in, and
  // an absolute path would churn the diff on every machine. An app's effects
  // are read at runtime and record the absolute path, which is what a
  // diagnostic can actually be clicked through to.
  sourceOf: (file: string) => string,
): string {
  const manifest = {
    schema: SCHEMA,
    slangc: SLANG_VERSION,
    kind,
    preludeSha256: preludeSha,
    effects: compiled.map((c, index) => ({
      // For built-ins this index IS `gpui::effect_id`; for app effects the host
      // allocates ids after the built-ins in exactly this order.
      id: index,
      name: c.effect.name,
      struct: c.effect.struct,
      doc: c.effect.doc,
      backdrop: c.effect.backdrop,
      hlsl: `${basename(c.file).replace(/\.slang$/, "")}.hlsl`,
      source: sourceOf(c.file),
      sourceSha256: c.sourceSha256,
      generatedSha256: sha256(c.hlsl),
      signature: c.effect.signature,
      backdropRadiusParam: c.effect.backdropRadiusParam,
      bleedParam: c.effect.bleedParam,
      animatedParam: c.effect.animatedParam,
      params: c.effect.params,
      bindings: c.reflection,
    })),
  };
  return `${JSON.stringify(manifest, null, 2)}\n`;
}

// ---------------------------------------------------------------------------
// built-in mode
// ---------------------------------------------------------------------------

function builtins(slangc: string, check: boolean) {
  const found = discover(BUILTIN_SHADERS);
  const byName = new Map(found.map((s) => [basename(s.file).replace(/\.slang$/, ""), s]));
  const missing = BUILTIN_ORDER.filter((n) => !byName.has(n));
  const extra = [...byName.keys()].filter((n) => !BUILTIN_ORDER.includes(n));
  if (missing.length || extra.length) {
    throw new Error(
      "the built-in effect set is a four-way contract with `gpui::effect_id`, the " +
        "gpui_windows pipeline table and crates/vn-effects.\n" +
        `  expected: ${BUILTIN_ORDER.join(", ")}\n` +
        (missing.length ? `  missing:  ${missing.join(", ")}\n` : "") +
        (extra.length
          ? `  extra:    ${extra.join(", ")}\n` +
            "  (adding a built-in means editing BUILTIN_ORDER here, gpui::effect_id and\n" +
            "   EffectShader::ALL in gpui_windows — an APP effect needs none of that;\n" +
            "   see docs/SHADERS-AUTHORING.md)\n"
          : ""),
    );
  }

  // Ordered by `gpui::effect_id`, not alphabetically.
  const ordered = BUILTIN_ORDER.map((n) => byName.get(n)!);
  const compiled = compileAll(slangc, ordered, [MODULES], (s) =>
    `crates/vn-effects/shaders/${basename(s.file)}+wrapper`,
  );
  for (const c of compiled) {
    const stem = basename(c.file).replace(/\.slang$/, "");
    if (c.effect.name !== stem) {
      throw new Error(
        `${basename(c.file)}: [VnEffect("${c.effect.name}")] does not match the file name — a ` +
          "built-in's file name is part of the pipeline-table contract",
      );
    }
  }

  const preludeSha = sha256(readFileSync(join(MODULES, "vn.slang"), "utf8"));
  const wanted: Array<[string, string]> = [
    ...compiled.map((c) => [
      join(GENERATED, `${basename(c.file).replace(/\.slang$/, "")}.hlsl`),
      c.hlsl,
    ] as [string, string]),
    [
      join(GENERATED, "effects.json"),
      manifestText("builtin", compiled, preludeSha, (f) =>
        `crates/vn-effects/shaders/${basename(f)}`,
      ),
    ],
  ];

  if (check) {
    const stale = wanted.filter(([path, text]) =>
      !existsSync(path) || readFileSync(path, "utf8") !== text,
    );
    if (stale.length) {
      console.error("generated shader artifacts are stale — run `bun shaders.ts`:");
      for (const [path] of stale) console.error(`  ${path}`);
      process.exit(1);
    }
    console.log(`shaders: ${compiled.length} built-in effect(s) up to date`);
    return;
  }

  mkdirSync(GENERATED, { recursive: true });
  for (const [path, text] of wanted) writeFileSync(path, text);
  // manifest.json was schema 3's name for the same artifact; effects.json
  // supersedes it and carries the parameter schema too.
  const legacy = join(GENERATED, "manifest.json");
  if (existsSync(legacy)) rmSync(legacy, { force: true });
  console.log(
    `shaders: wrote ${compiled.length} built-in effect(s) -> ${GENERATED}\n  ` +
      compiled.map((c) => c.effect.signature).join("\n  "),
  );
}

// ---------------------------------------------------------------------------
// app mode
// ---------------------------------------------------------------------------

function builtinNames(): string[] {
  const path = join(GENERATED, "effects.json");
  if (!existsSync(path)) return BUILTIN_ORDER;
  try {
    return JSON.parse(readFileSync(path, "utf8")).effects.map((e: any) => e.name);
  } catch {
    return BUILTIN_ORDER;
  }
}

function appOnce(slangc: string, shadersDir: string, out: string): number {
  const sources = discover(shadersDir);
  if (sources.length === 0) {
    throw new Error(`no .slang sources under ${shadersDir}`);
  }
  const reserved = new Set(builtinNames());
  const compiled = compileAll(
    slangc,
    sources,
    // The app's own shaders/ dir is an include path too, so an app can keep
    // shared helpers in `shaders/<something>.slang` and `import` them.
    [MODULES, shadersDir],
    (s) => s.file.replace(/\\/g, "/"),
  );

  const seen = new Map<string, string>();
  for (const c of compiled) {
    if (reserved.has(c.effect.name)) {
      throw new Error(
        `${basename(c.file)}: [VnEffect("${c.effect.name}")] shadows a built-in effect. ` +
          "Built-ins win, so pick another name.",
      );
    }
    const previous = seen.get(c.effect.name);
    if (previous) {
      throw new Error(
        `[VnEffect("${c.effect.name}")] is declared twice: ${basename(previous)} and ` +
          basename(c.file),
      );
    }
    seen.set(c.effect.name, c.file);
  }

  mkdirSync(out, { recursive: true });
  for (const c of compiled) {
    writeFileSync(join(out, basename(c.file).replace(/\.slang$/, ".hlsl")), c.hlsl);
  }
  const preludeSha = sha256(readFileSync(join(MODULES, "vn.slang"), "utf8"));
  // effects.json is written LAST: the host reads it to decide what to load, so
  // once its mtime moves every .hlsl it names is already on disk.
  writeFileSync(
    join(out, "effects.json"),
    manifestText("user", compiled, preludeSha, (f) => f.replace(/\\/g, "/")),
  );
  console.log(
    `shaders: wrote ${compiled.length} app effect(s) -> ${out}\n  ` +
      compiled.map((c) => c.effect.signature).join("\n  "),
  );
  return compiled.length;
}

function app(slangc: string, appDir: string, out: string, watchMode: boolean) {
  const shadersDir = join(appDir, "shaders");
  if (!existsSync(shadersDir) || !statSync(shadersDir).isDirectory()) {
    throw new Error(
      `${appDir} has no shaders/ directory. An app's effects live in ` +
        `${shadersDir}/<name>.slang — see docs/SHADERS-AUTHORING.md`,
    );
  }

  const once = () => {
    try {
      appOnce(slangc, shadersDir, out);
      return true;
    } catch (error) {
      console.error(`${error instanceof Error ? error.message : error}`);
      return false;
    }
  };

  const ok = once();
  if (!watchMode) {
    if (!ok) process.exit(1);
    return;
  }

  console.log(`shaders: watching ${shadersDir}`);
  let timer: ReturnType<typeof setTimeout> | null = null;
  watch(shadersDir, { recursive: true }, (_event, file) => {
    if (file && !String(file).endsWith(".slang")) return;
    if (timer) clearTimeout(timer);
    // Editors write in bursts; coalesce.
    timer = setTimeout(() => {
      timer = null;
      // A compile error keeps the LAST GOOD artifacts on disk and prints the
      // diagnostic. The running app is untouched; nothing dies over a typo.
      if (once()) {
        console.log(
          "shaders: effect artifacts updated — restart the app to load them " +
            "(pipelines are built once, at renderer construction)",
        );
      }
    }, 60);
  });
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

function flag(name: string): string | undefined {
  const index = process.argv.indexOf(`--${name}`);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function main() {
  const slangc = findSlangc();
  const appDir = flag("app");
  if (appDir) {
    const out = flag("out");
    if (!out) {
      throw new Error(
        "--app needs --out <dir>: the directory the compiled effects land in, which is what " +
          "the host reads via VN_EFFECTS_DIR. Conventionally <dist>/effects.",
      );
    }
    app(slangc, resolve(appDir), resolve(out), process.argv.includes("--watch"));
    return;
  }
  builtins(slangc, process.argv.includes("--check"));
}

main();
