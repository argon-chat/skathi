// Effect shader compiler: `.slang` -> one fragment stage per renderer target (`TARGETS`: HLSL for
// DirectX, WGSL for gpui_wgpu, MSL for Metal) + a reflected schema (`effects.json`), through redslangc.
// `sk build`, `sk dev` and `sk check` run it over an app's shaders/*.slang; scripts/shaders.ts over the
// built-ins.
//
// An effect author writes one struct (`[SkEffect("ripple")] struct Ripple : IEffect { ... }`).
// The reflection of its attributes, field order and types is the single source of truth for the CSS
// function name, argument order, packing slots, types, defaults and flags, so a new effect needs no
// registration in Rust or JS.
//
// The vertex stage is hand-written per renderer (gpui_windows/src/effects.hlsl,
// gpui_wgpu/src/shaders_effects.wgsl): gpui clips with `float4 : SV_ClipDistance` and no spelling of
// that survives both Slang's front end and fxc.

import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { availableParallelism } from "node:os";
import { basename, join, relative } from "node:path";
import { PIN, ShaderCompilerError } from "./redslangc";

/// Bumped when the prelude or generated wrapper changes codegen or schema shape; rides in every
/// header and effects.json. Rust refuses an effects.json with an unknown schema.
export const SCHEMA = 14;

export const PRELUDE = join(import.meta.dir, "sk.slang");

/// The standard library beside the prelude: `effects/sk/color.slang` is `import sk.color;`. Served by
/// name rather than through a search path, so `#line` directives never carry a machine's path.
export const STD_MODULES: Array<[name: string, file: string]> = readdirSync(join(import.meta.dir, "sk"))
  .filter((f) => f.endsWith(".slang"))
  .sort()
  .map((f) => [`sk.${f.slice(0, -".slang".length)}`, join(import.meta.dir, "sk", f)]);

/// The built-in numbering is a contract: this list, `gpui::effect_id`, the pipeline tables in
/// `gpui_windows` and `gpui_wgpu`, and the ids in `crates/sk-effects/generated/effects.json`. It is not alphabetical.
export const BUILTIN_EFFECTS = ["frost", "noise", "glow"];

/// Floats per instance (`EffectQuad::params`); raising it widens every effect quad.
const PARAM_SLOTS = 32;

const ENTRY = "effect_fragment";

export type TargetName = "hlsl" | "wgsl" | "metal";

/// One renderer's shading language. `name` is the effects.json key naming each effect's file and the
/// file's extension, which is how a renderer finds its own.
interface Target {
  name: TargetName;
  /// redslangc's `--target`.
  spec: string;
  /// The last lines of the generated header: what the renderer adds to this fragment stage.
  note: () => string;
  /// Refuses code the renderer's pipeline layout cannot run.
  check: (code: string, file: string) => void;
}

const TARGETS: Target[] = [
  {
    name: "hlsl",
    spec: "hlsl:sm_5_0",
    note: () =>
      `// Compiled at ps_5_0 (effects only — the core gpui shaders stay at ps_4_1).\n` +
      `// The VERTEX half lives in gpui_windows/src/effects.hlsl and is shared by\n` +
      `// every effect; Slang cannot author it (SV_ClipDistance, see that file).\n`,
    check: () => {},
  },
  {
    name: "wgsl",
    spec: "wgsl",
    note: () =>
      `// The VERTEX half lives in gpui_wgpu/src/shaders_effects.wgsl and is shared\n` +
      `// by every effect; it clips the quad to the content mask.\n`,
    check: checkWgslBindings,
  },
  {
    name: "metal",
    spec: "metal",
    note: () => METAL_NOTE,
    check: checkMetal,
  },
];

// Group 0 the globals, 1 the instance buffer, 2 the backdrop, its sampler and the snapshot, the
// `SkTexture` images (3..6), the element's content (7) and history (8), the images' repeating
// sampler (9), and the snapshot without underlay sources (10).
const WGSL_BINDINGS = new Set([
  "0:0",
  "1:0",
  "2:0",
  "2:1",
  "2:2",
  "2:3",
  "2:4",
  "2:5",
  "2:6",
  "2:7",
  "2:8",
  "2:9",
  "2:10",
]);

/// The renderer builds one pipeline layout for every effect; a resource outside it would fail at
/// pipeline creation on the user's machine, so refuse it here.
function checkWgslBindings(code: string, file: string) {
  for (const m of code.matchAll(/@binding\((\d+)\)\s*@group\((\d+)\)\s*var(?:<[^>]*>)?\s+(\w+)/g)) {
    if (!WGSL_BINDINGS.has(`${m[2]}:${m[1]}`)) {
      throw new SchemaError(
        `${basename(file)}: \`${m[3]}\` needs @group(${m[2]}) @binding(${m[1]}) in wgsl, which the ` +
          "engine does not provide — an effect reads only `EffectInput` and its own parameters, it " +
          "cannot declare textures, samplers or buffers of its own",
      );
    }
  }
}

/// The prelude's resources on Metal, where Slang keeps the HLSL registers (b0, t1, t0, t2, t3..t8, t13, s1, s2). One
/// argument table for every effect, so the renderer binds these and nothing else.
const METAL_BINDINGS: Array<[at: string, name: string, what: string]> = [
  ["buffer(0)", "GlobalParams", "constant, the 48-byte globals"],
  ["buffer(1)", "sk_effect_quads", "device EffectQuad[], 264 bytes each"],
  ["texture(0)", "sk_backdrop_input", "the blurred backdrop"],
  ["texture(2)", "sk_backdrop_original", "the render-target snapshot"],
  ["texture(13)", "sk_backdrop_page", "the snapshot without underlays"],
  ["texture(7)", "sk_content", "the element's content"],
  ["texture(8)", "sk_history", "the effect's history"],
  ["sampler(1)", "sk_clamp_sampler", "clamp to edge"],
  ["texture(3)", "sk_texture0", "the first SkTexture"],
  ["texture(4)", "sk_texture1", "the second SkTexture"],
  ["texture(5)", "sk_texture2", "the third SkTexture"],
  ["texture(6)", "sk_texture3", "the fourth SkTexture"],
  ["sampler(2)", "sk_repeat_sampler", "repeat"],
];

/// Refuses Metal output whose entry point or arguments are not the ones the renderer provides.
function checkMetal(code: string, file: string): void {
  const entry = new RegExp(`\\[\\[fragment\\]\\]\\s+\\w+\\s+${ENTRY}\\s*\\(([^{]*)\\)\\s*\\{`).exec(code);
  if (!entry) throw new SchemaError(`${basename(file)}: the Metal output has no \`[[fragment]] ${ENTRY}\``);
  if (!/\[\[flat\]\]\s+uint\s+sk_id_\d+\s+\[\[user\(TEXCOORD\)\]\]/.test(code)) {
    throw new SchemaError(`${basename(file)}: the Metal stage_in is not \`[[flat]] uint sk_id [[user(TEXCOORD)]]\``);
  }
  for (const m of entry[1]!.matchAll(/(\w+)\s*\[\[\s*((?:buffer|texture|sampler)\(\d+\))\s*\]\]/g)) {
    const name = m[1]!.replace(/_\d+$/, "");
    if (!METAL_BINDINGS.some(([at, n]) => n === name && at === m[2])) {
      throw new SchemaError(
        `${basename(file)}: \`${m[1]}\` is bound to [[${m[2]}]] in metal, which the engine does not provide — ` +
          "an effect reads only `EffectInput` and its own parameters, it cannot declare textures, samplers or " +
          "buffers of its own",
      );
    }
  }
}

const METAL_NOTE =
  `// Entry point \`[[fragment]] ${ENTRY}\`. Its [[stage_in]] is the vertex half's\n` +
  `// \`[[flat]] uint sk_id [[user(TEXCOORD)]]\`, beside [[position]]; it writes a\n` +
  `// premultiplied [[color(0)]]. Arguments, the same for every effect:\n` +
  METAL_BINDINGS.map(([at, name, what]) => `//   [[${at}]]`.padEnd(20) + `${name.padEnd(22)}${what}\n`).join("") +
  `// The VERTEX half is the Metal renderer's own, shared by every effect.\n`;

type ParamKind = "number" | "length" | "color" | "texture";

type ParamSchema = {
  name: string;
  kind: ParamKind;
  /// A texture's first slot holds its width and the next its height, which the host writes.
  slot: number;
  required: boolean;
  /// A texture's is what it reads as while missing, straight RGBA.
  default: [number, number, number, number];
  range: [number, number] | null;
  /// A texture's binding, 0..3 (t3..t6), in declaration order.
  texture?: number;
  /// A texture sampled as stored, without premultiplying ([SkLinear]).
  linear?: boolean;
};

/// `SkTexture` fields per effect: the bindings the renderers provide.
const TEXTURE_SLOTS = 4;

export type EffectSchema = {
  name: string;
  struct: string;
  doc: string | null;
  backdrop: boolean;
  /// `[SkContent]`: drawn in place of its element's own painting, which it reads.
  content: boolean;
  /// `[SkHistory]`: keeps state per element from frame to frame.
  history: boolean;
  /// `[SkBackdropLinear]`: reads its backdrop in linear, extended light and returns linear light.
  backdropLinear: boolean;
  params: ParamSchema[];
  backdropRadiusParam: number | null;
  /// `[SkBackdropReach]`: how far outside its quad the effect samples the backdrop.
  backdropReachParam: number | null;
  bleedParam: number | null;
  animatedParam: number | null;
  /// `[SkPointer]`'s first slot: the host writes the pointer there; not a CSS argument.
  pointerParam: number | null;
  /// `[SkPointerTimes]`'s first slot, host-filled like `pointerParam`.
  pointerTimesParam: number | null;
  /// Scalar [SkQuality] tier supplied by the painter.
  qualityParam: number | null;
  /// Every slot of an `[SkAnimatedFor]` field: times on the effect clock.
  clockSlots: number[];
  /// The longest `[SkAnimatedFor]` window, in seconds.
  clockWindow: number | null;
  signature: string;
};

// Two wrappers. The probe makes the author's struct a reflectable parameter without reading a
// field, so Slang strips the cbuffer from the HLSL while reflection still describes it. The real
// one unpacks the slots into the named fields.

const WRAPPER_BANNER = `
// ===========================================================================
//  engine-generated wrapper — DO NOT EDIT, DO NOT COPY INTO AN EFFECT
//  (appended by the effect compiler, schema ${SCHEMA})
//  Appended, never prepended, so diagnostics keep YOUR line numbers.
// ===========================================================================
`;

function probeWrapper(structName: string): string {
  return `${WRAPPER_BANNER}
// b3 is unused by gpui (b0 = globals, b1 = batch params). This declaration is
// never read, so it does not reach the HLSL; it exists solely so that the
// reflection describes \`${structName}\` and its user-defined attributes.
[[vk::binding(0, 3)]]
ConstantBuffer<${structName}> sk_reflect : register(b3);

[shader("fragment")]
float4 sk_reflect_probe(float4 sk_position : SV_Position) : SV_Target
{
    return float4(0.0, 0.0, 0.0, 0.0);
}
`;
}

const slotExpr = (slot: number): string => `sk_in.param(${slot}u)`;

function loadExpr(field: Field): string {
  if (field.texture) {
    const { index, fallback } = field.texture;
    return (
      `sk_texture(${index}u, float2(${slotExpr(field.slot)}, ${slotExpr(field.slot + 1)}), ` +
      `float4(${fallback.join(", ")}))`
    );
  }
  if (field.components === 1) return slotExpr(field.slot);
  const parts: string[] = [];
  for (let c = 0; c < field.components; c += 1) parts.push(slotExpr(field.slot + c));
  return `float${field.components}(${parts.join(", ")})`;
}

function realWrapper(effect: EffectSchema, fields: Field[]): string {
  const assignments = fields.map((field) => `    sk_material.${field.name} = ${loadExpr(field)};`).join("\n");
  return `${WRAPPER_BANNER}
// Kept for the record: reflection reads this declaration, every backend
// strips it (nothing references it), and the renderer never binds it.
[[vk::binding(0, 3)]]
ConstantBuffer<${effect.struct}> sk_reflect : register(b3);

// The varyings of the engine-owned vertex stage (gpui_windows/src/effects.hlsl,
// gpui_wgpu/src/shaders_effects.wgsl), in their order. Separate parameters:
// Slang's WGSL keeps \`nointerpolation\` on a struct it copies them into, and
// naga rejects an interpolated member without a location.
[shader("fragment")]
float4 ${ENTRY}(nointerpolation uint sk_id : TEXCOORD0, float4 sk_position : SV_Position) : SV_Target
{
    EffectInput sk_in = sk_begin(sk_id, sk_position);

    // Parameter unpacking, generated from the reflection of
    // \`${effect.struct}\`: field order IS slot order IS CSS argument order.
    ${effect.struct} sk_material;
${assignments || "    // (this effect declares no parameters)"}

    return sk_finish(sk_material, sk_in);
}
`;
}

/// One compiler message, at the author's line numbers.
export interface Problem {
  severity: "error" | "warning" | "note";
  /// The effect's path; `sk.slang` for the prelude, `<engine wrapper>` for the generated tail.
  file: string;
  line: number;
  column: number;
  message: string;
}

export class EffectsError extends Error {
  constructor(readonly problems: Problem[]) {
    super(formatProblems(problems.filter((p) => p.severity === "error")));
  }
}

class SchemaError extends Error {}

export function formatProblems(problems: Problem[], cwd: string = process.cwd()): string {
  return problems
    .map((p) => {
      const file = p.file.startsWith("<") || p.file === "sk.slang" ? p.file : relative(cwd, p.file) || p.file;
      const at = p.line ? `:${p.line}${p.column ? `:${p.column}` : ""}` : "";
      return `${file.replace(/\\/g, "/")}${at}: ${p.severity}: ${p.message}`;
    })
    .join("\n");
}

const sha256 = (text: string): string => createHash("sha256").update(text).digest("hex");

/// A Windows checkout may hold CRLF; hashes and the generated headers must match a Linux one.
const readText = (file: string): string => readFileSync(file, "utf8").replace(/\r\n/g, "\n");

interface ReflType {
  kind?: string;
  name?: string;
  scalarType?: string;
  elementType?: ReflType;
  elementCount?: number | string;
  userAttribs?: Attribute[];
  fields?: ReflField[];
}
interface ReflField {
  name?: unknown;
  type?: ReflType;
  userAttribs?: Attribute[];
}
interface ReflParam {
  name?: string;
  type?: ReflType;
  binding?: { kind?: unknown; index?: unknown };
}
interface Reflection {
  parameters?: ReflParam[];
}

interface Message {
  severity: Problem["severity"];
  file: string | null;
  line: number | null;
  column: number | null;
  message: string;
}

interface RunResult {
  ok: boolean;
  code: Partial<Record<TargetName, string>>;
  reflection: Reflection | null;
  messages: Message[];
  diagnostics: string;
}

interface Request {
  compiler: string;
  source: string;
  sourceName: string;
  module: string;
  includes: string[];
  targets: Target[];
  entry: string;
}

async function redslangc(r: Request): Promise<RunResult> {
  const args = [
    "-",
    "--module",
    r.module,
    "--source-name",
    r.sourceName,
    "--import",
    `sk=${PRELUDE}`,
    ...STD_MODULES.flatMap(([name, file]) => ["--import", `${name}=${file}`]),
    ...r.includes.flatMap((dir) => ["-I", dir]),
    ...r.targets.flatMap((t) => ["-t", t.spec]),
    "-e",
    r.entry,
    // Pinned even though no effect takes a matrix yet: Slang 2026.14 flipped this default.
    "--matrix-layout",
    "row-major",
  ];
  const child = Bun.spawn([r.compiler, ...args], {
    stdin: new TextEncoder().encode(r.source),
    stdout: "pipe",
    stderr: "pipe",
    windowsHide: true,
  });
  const [stdout, stderr, status] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ]);
  if (status !== 0 && status !== 1) {
    throw new ShaderCompilerError(`${r.compiler} failed (exit ${status}): ${stderr.trim() || stdout.trim()}`);
  }
  let json: Record<string, unknown>;
  try {
    json = JSON.parse(stdout) as Record<string, unknown>;
  } catch {
    throw new ShaderCompilerError(`${r.compiler} printed no JSON (exit ${status}): ${stderr.trim()}`);
  }
  const code: Partial<Record<TargetName, string>> = {};
  for (const t of r.targets) {
    const text = json[t.name];
    if (typeof text === "string") code[t.name] = text;
  }
  return {
    ok: json.ok === true,
    code,
    reflection: (json.reflection as Reflection | null) ?? null,
    messages: Array.isArray(json.messages) ? (json.messages as Message[]) : [],
    diagnostics: typeof json.diagnostics === "string" ? json.diagnostics : "",
  };
}

/// The register each top-level parameter landed on: the contract with the D3D11 renderer
/// (t0 backdrop, t1 instance buffer, t2 snapshot, b0 globals, s1 clamp).
function bindings(reflection: Reflection | null): Array<Record<string, unknown>> {
  const params = Array.isArray(reflection?.parameters) ? reflection.parameters : [];
  return params
    .filter((p) => p?.name !== "sk_reflect")
    .map((p) => ({ name: p?.name, kind: p?.binding?.kind, index: p?.binding?.index }));
}

type Attribute = { name: string; arguments: unknown[] };
type Field = {
  name: string;
  components: number;
  slot: number;
  /// An `SkTexture`: its binding and the colour it reads as while missing, as `sample` returns it.
  texture?: { index: number; fallback: [number, number, number, number] };
};

function attr(attrs: Attribute[], name: string): Attribute | undefined {
  return attrs.find((a) => a.name === name);
}

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
  if (numbers.length === 0) throw new SchemaError(`${where}: empty [SkDefault]`);
  if (numbers.length === 1) {
    for (let c = 0; c < components; c += 1) out[c] = numbers[0]!;
    return out;
  }
  if (numbers.length !== components) {
    throw new SchemaError(
      `${where}: [SkDefault] has ${numbers.length} values but the field has ${components} component(s)`,
    );
  }
  for (let c = 0; c < components; c += 1) out[c] = numbers[c]!;
  return out;
}

/// Whether a `components`-wide field may start at `slot`: free, in range, and not straddling a
/// 16-byte (four-slot) boundary (HLSL's rule).
function fits(used: boolean[], slot: number, components: number): boolean {
  if (slot < 0 || slot + components > PARAM_SLOTS) return false;
  if (components > 1 && Math.floor(slot / 4) !== Math.floor((slot + components - 1) / 4)) {
    return false;
  }
  for (let c = 0; c < components; c += 1) if (used[slot + c]) return false;
  return true;
}

function allocate(used: boolean[], components: number): number {
  for (let slot = 0; slot + components <= PARAM_SLOTS; slot += 1) {
    if (fits(used, slot, components)) return slot;
  }
  return -1;
}

const isTexture = (type: ReflType | undefined): boolean => type?.kind === "struct" && type.name === "SkTexture";

function fieldComponents(type: ReflType | undefined, where: string): number {
  if (isTexture(type)) return 2;
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
    `${where}: unsupported parameter type \`${type?.name ?? type?.kind ?? "?"}\` — a parameter must be ` +
      "`float`, `float2`, `float3`, `float4` or `SkTexture` (an image: matrices, arrays, raw " +
      "textures and samplers cannot ride in the 32-float parameter block)",
  );
}

const COMPONENT_SUFFIX = ["x", "y", "z", "w"];

const HOST_FILLED_REFUSES = [
  "SkDefault",
  "SkRange",
  "SkLength",
  "SkColor",
  "SkOptional",
  "SkAnimated",
  "SkBleed",
  "SkBackdropRadius",
  "SkBackdropReach",
];

function buildSchema(
  reflection: Reflection | null,
  structName: string,
  source: string,
): {
  effect: EffectSchema;
  fields: Field[];
} {
  const param = (reflection?.parameters ?? []).find((p) => p?.name === "sk_reflect");
  const element = param?.type?.elementType;
  if (!element || element.kind !== "struct") {
    throw new SchemaError(
      `${source}: the compiler reflected no struct for \`${structName}\` — this is an internal ` +
        "error in the effect toolchain, not in your shader",
    );
  }

  const structAttrs: Attribute[] = element.userAttribs ?? [];
  const effectAttr = attr(structAttrs, "SkEffect");
  if (!effectAttr) {
    throw new SchemaError(
      `${source}: \`struct ${structName}\` has no [SkEffect("<name>")] attribute — that ` +
        "attribute is what gives the effect its CSS function name",
    );
  }
  const name = String(effectAttr.arguments?.[0] ?? "");
  if (!/^[a-z][a-z0-9-]*$/.test(name)) {
    throw new SchemaError(
      `${source}: [SkEffect("${name}")] — an effect name must be a lowercase CSS ident ` +
        "(`[a-z][a-z0-9-]*`), because it becomes a CSS function name",
    );
  }
  const docAttr = attr(structAttrs, "SkDoc");

  const params: ParamSchema[] = [];
  const fields: Field[] = [];
  let backdropRadiusParam: number | null = null;
  let backdropReachParam: number | null = null;
  let bleedParam: number | null = null;
  let animatedParam: number | null = null;
  let pointerParam: number | null = null;
  let pointerTimesParam: number | null = null;
  let qualityParam: number | null = null;
  const clockSlots: number[] = [];
  let clockWindow: number | null = null;
  let textures = 0;
  let backdrop = attr(structAttrs, "SkBackdrop") !== undefined;
  // Taken float slots: `[SkSlot]` runs are reserved first, then fields pack in declaration order.
  const used: boolean[] = new Array(PARAM_SLOTS).fill(false);
  const pinned = new Map<string, number>();
  for (const field of element.fields ?? []) {
    const slotAttr = attr(field.userAttribs ?? [], "SkSlot");
    if (!slotAttr) continue;
    const where = `${source}: \`${structName}.${String(field.name)}\``;
    const components = fieldComponents(field.type, where);
    const slot = Number(slotAttr.arguments?.[0]);
    if (!Number.isInteger(slot) || !fits(used, slot, components)) {
      throw new SchemaError(
        `${where}: [SkSlot(${slotAttr.arguments?.[0]})] is not a legal home for a ` +
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

    const isColor = attr(attrs, "SkColor") !== undefined;
    const isLength = attr(attrs, "SkLength") !== undefined;
    const optional = attr(attrs, "SkOptional") !== undefined;
    const rangeAttr = attr(attrs, "SkRange");
    const defaultAttr = attr(attrs, "SkDefault");

    if (isColor && components !== 4) {
      throw new SchemaError(`${where}: [SkColor] is only legal on a float4 (RGBA)`);
    }
    if (isColor && isLength) {
      throw new SchemaError(`${where}: [SkColor] and [SkLength] are mutually exclusive`);
    }
    if (optional && components > 1 && !isColor && !isTexture(field.type)) {
      // A bare vector expands to one CSS argument per component and optional parameters are
      // skipped one argument at a time, so half a `float2` could be filled in: refuse.
      throw new SchemaError(
        `${where}: [SkOptional] is not allowed on a float${components} — it expands to ` +
          `${components} CSS arguments and only some of them could be omitted. Split it into ` +
          "scalar fields, or drop [SkOptional].",
      );
    }

    let slot = pinned.get(fieldName) ?? -1;
    if (slot < 0) {
      slot = allocate(used, components);
      if (slot < 0) {
        throw new SchemaError(
          `${where}: no room left in the parameter block. An effect quad carries ` +
            `${PARAM_SLOTS} floats (EffectQuad::params) and a <color> costs 4; ` +
            "drop or merge a parameter.",
        );
      }
      for (let c = 0; c < components; c += 1) used[slot + c] = true;
    }

    const linear = attr(attrs, "SkLinear") !== undefined;
    if (isTexture(field.type)) {
      const refused = [
        ...HOST_FILLED_REFUSES.filter((a) => a !== "SkDefault" && a !== "SkOptional"),
        "SkAnimatedFor",
        "SkPointer",
        "SkPointerTimes",
        "SkQuality",
      ].find((a) => attr(attrs, a) !== undefined);
      if (refused) throw new SchemaError(`${where}: [${refused}] cannot go on an SkTexture`);
      if (textures === TEXTURE_SLOTS) {
        throw new SchemaError(`${where}: an effect samples at most ${TEXTURE_SLOTS} SkTexture images`);
      }
      const fallback = defaultAttr
        ? parseDefault(String(defaultAttr.arguments?.[0] ?? ""), 4, where)
        : ([0, 0, 0, 0] as [number, number, number, number]);
      const [r, g, b, a] = fallback;
      fields.push({
        name: fieldName,
        components,
        slot,
        texture: { index: textures, fallback: linear ? fallback : [r * a, g * a, b * a, a] },
      });
      params.push({
        name: fieldName,
        kind: "texture",
        slot,
        required: !optional,
        default: fallback,
        range: null,
        texture: textures,
        linear,
      });
      textures += 1;
      continue;
    }
    if (linear) throw new SchemaError(`${where}: [SkLinear] is only legal on an SkTexture`);
    fields.push({ name: fieldName, components, slot });

    const animatedFor = attr(attrs, "SkAnimatedFor");
    if (animatedFor) {
      const seconds = Number(animatedFor.arguments?.[0]);
      if (!Number.isFinite(seconds) || seconds <= 0) {
        throw new SchemaError(
          `${where}: [SkAnimatedFor(${animatedFor.arguments?.[0]})] needs a positive number of seconds`,
        );
      }
      clockWindow = Math.max(clockWindow ?? 0, seconds);
      for (let c = 0; c < components; c += 1) clockSlots.push(slot + c);
    }

    const host = (["SkPointer", "SkPointerTimes", "SkQuality"] as const).filter((a) => attr(attrs, a) !== undefined);
    if (host.length > 1) throw new SchemaError(`${where}: host attributes ${host.join(", ")} are mutually exclusive`);
    if (host[0]) {
      const kind = host[0];
      const wanted = { SkPointer: 2, SkPointerTimes: 3, SkQuality: 1 }[kind];
      if (components !== wanted)
        throw new SchemaError(`${where}: [${kind}] requires float${wanted === 1 ? "" : wanted}`);
      if (kind === "SkQuality" && animatedFor)
        throw new SchemaError(`${where}: [SkQuality] cannot be an animation clock`);
      const css = HOST_FILLED_REFUSES.find((a) => attr(attrs, a) !== undefined);
      if (css) {
        throw new SchemaError(
          `${where}: [${css}] cannot go on a [${kind}] parameter — the host fills it, it is not a CSS argument`,
        );
      }
      const current = { SkPointer: pointerParam, SkPointerTimes: pointerTimesParam, SkQuality: qualityParam }[kind];
      if (current !== null) throw new SchemaError(`${where}: [${kind}] appears more than once in \`${structName}\``);
      if (kind === "SkPointer") pointerParam = slot;
      else if (kind === "SkPointerTimes") pointerTimesParam = slot;
      else qualityParam = slot;
      continue;
    }

    const defaults = defaultAttr
      ? parseDefault(String(defaultAttr.arguments?.[0] ?? ""), components, where)
      : ([0, 0, 0, 0] as [number, number, number, number]);
    const range = rangeAttr
      ? ([Number(rangeAttr.arguments?.[0]), Number(rangeAttr.arguments?.[1])] as [number, number])
      : null;

    if (isColor) {
      params.push({ name: fieldName, kind: "color", slot, required: !optional, default: defaults, range: null });
    } else {
      for (let c = 0; c < components; c += 1) {
        params.push({
          name: components === 1 ? fieldName : `${fieldName}.${COMPONENT_SUFFIX[c]}`,
          kind: isLength ? "length" : "number",
          slot: slot + c,
          required: !optional,
          default: [defaults[c]!, 0, 0, 0],
          range,
        });
      }
    }

    const marker = (attrName: string, current: number | null, requires: string) => {
      if (attr(attrs, attrName) === undefined) return current;
      if (current !== null) {
        throw new SchemaError(`${where}: [${attrName}] appears more than once in \`${structName}\``);
      }
      if (components !== 1) {
        throw new SchemaError(`${where}: [${attrName}] is only legal on a scalar \`float\``);
      }
      if (requires === "length" && !isLength) {
        throw new SchemaError(`${where}: [${attrName}] requires [SkLength] on the same field`);
      }
      return slot;
    };
    backdropRadiusParam = marker("SkBackdropRadius", backdropRadiusParam, "length");
    backdropReachParam = marker("SkBackdropReach", backdropReachParam, "length");
    bleedParam = marker("SkBleed", bleedParam, "length");
    animatedParam = marker("SkAnimated", animatedParam, "number");
  }

  const backdropLinear = attr(structAttrs, "SkBackdropLinear") !== undefined;
  if (backdropRadiusParam !== null || backdropReachParam !== null || backdropLinear) backdrop = true;

  // Type-directed matching needs a skippable parameter's kind to differ from the next one's, else
  // `ripple(1)` is ambiguous.
  for (let i = 0; i < params.length; i += 1) {
    const cur = params[i]!;
    if (cur.required) continue;
    const nextParam = params[i + 1];
    if (nextParam && nextParam.kind === cur.kind) {
      throw new SchemaError(
        `${source}: \`${name}\` — optional parameter \`${cur.name}\` has the same CSS ` +
          `type (${cur.kind}) as the parameter after it (\`${nextParam.name}\`), so an ` +
          "argument in that position would be ambiguous. Make it required, reorder, or change " +
          "its type.",
      );
    }
  }

  const signature = `${name}(${params
    .map((p) => `<${p.name === p.kind ? p.kind : `${p.kind}:${p.name}`}>${p.required ? "" : "?"}`)
    .join(", ")})`;

  return {
    effect: {
      name,
      struct: structName,
      doc: docAttr ? String(docAttr.arguments?.[0] ?? "") : null,
      backdrop,
      content: attr(structAttrs, "SkContent") !== undefined,
      history: attr(structAttrs, "SkHistory") !== undefined,
      backdropLinear,
      params,
      backdropRadiusParam,
      backdropReachParam,
      bleedParam,
      animatedParam,
      pointerParam,
      pointerTimesParam,
      qualityParam,
      clockSlots,
      clockWindow,
      signature,
    },
    fields,
  };
}

export interface EffectSource {
  /// The `.slang` file.
  file: string;
  /// Its name in `#line` directives, diagnostics and the generated header: stable across machines.
  sourceName: string;
  struct: string;
  source: string;
}

/// The struct name must be known before compiling (the probe wrapper names it): one `struct <Name> : IEffect` per file.
function findStruct(file: string, source: string): string {
  const stripped = source.replace(/\/\/[^\n]*/g, "").replace(/\/\*[\s\S]*?\*\//g, "");
  const matches = [...stripped.matchAll(/\bstruct\s+([A-Za-z_]\w*)\s*:\s*(?:sk\.)?IEffect\b/g)];
  if (matches.length === 0) {
    throw new SchemaError(
      `${basename(file)}: no \`struct <Name> : IEffect\` found. An effect is a struct that ` +
        "conforms to the `IEffect` interface from the `sk` prelude (`import sk;`).",
    );
  }
  if (matches.length > 1) {
    throw new SchemaError(
      `${basename(file)}: ${matches.length} effect structs in one file ` +
        `(${matches.map((m) => m[1]).join(", ")}) — one effect per file`,
    );
  }
  return matches[0]![1]!;
}

export const stemOf = (file: string): string => basename(file).replace(/\.slang$/, "");

/// Where a schema complaint about `Struct.field` points: the field's declaration.
function locate(source: string, message: string): { line: number; column: number } {
  const field = /`\w+\.(\w+)`/.exec(message)?.[1];
  const struct = /`(?:struct )?(\w+)`/.exec(message)?.[1];
  const lines = source.split("\n");
  const find = (re: RegExp) => lines.findIndex((l) => re.test(l.replace(/\/\/.*$/, "")));
  let at = field ? find(new RegExp(`\\b${field}\\s*;`)) : -1;
  if (at < 0 && struct) at = find(new RegExp(`\\bstruct\\s+${struct}\\b`));
  if (at < 0) return { line: 0, column: 0 };
  const column = (field ? lines[at]!.search(new RegExp(`\\b${field}\\b`)) : lines[at]!.indexOf("struct")) + 1;
  return { line: at + 1, column: Math.max(column, 1) };
}

function schemaProblem(file: string, source: string | null, error: SchemaError): Problem {
  const message = error.message.replace(/^[^:`]+\.slang: /, "");
  const where = source === null ? { line: 0, column: 0 } : locate(source, message);
  return { severity: "error", file, ...where, message };
}

const DECLARES_EFFECT = /\[\s*SkEffect\s*\(/;

function slangFiles(dir: string): string[] {
  return (readdirSync(dir, { recursive: true }) as string[])
    .filter((f) => f.endsWith(".slang"))
    .map((f) => join(dir, f))
    .sort();
}

/// The effects under `dir`: every `.slang` that declares an `[SkEffect]`; the rest are modules they import.
export function discover(
  dir: string,
  sourceName: (file: string) => string,
): { sources: EffectSource[]; problems: Problem[] } {
  const sources: EffectSource[] = [];
  const problems: Problem[] = [];
  const stems = new Map<string, string>();
  for (const file of slangFiles(dir)) {
    const source = readText(file);
    if (!DECLARES_EFFECT.test(source)) continue;
    const f = relative(dir, file);
    try {
      if (stemOf(file) === "sk") {
        throw new SchemaError(`${f}: an effect cannot be named sk.slang — \`sk\` is the prelude's module`);
      }
      const other = stems.get(stemOf(file));
      if (other) throw new SchemaError(`${f}: ${other} is an effect file with the same name; rename one`);
      stems.set(stemOf(file), f);
      sources.push({ file, sourceName: sourceName(file), struct: findStruct(file, source), source });
    } catch (e) {
      if (!(e instanceof SchemaError)) throw e;
      problems.push(schemaProblem(file, source, e));
    }
  }
  return { sources, problems };
}

export type Compiled = {
  file: string;
  effect: EffectSchema;
  /// The generated source per target, in `TARGETS` order.
  outputs: Array<{ target: TargetName; code: string }>;
  reflection: unknown;
  sourceSha256: string;
  warnings: Problem[];
};

function problemsOf(messages: Message[], s: EffectSource, userLines: number): Problem[] {
  return messages.map((m) => {
    const line = m.line ?? 0;
    const ours = m.file === s.sourceName;
    const inUser = ours && line <= userLines;
    return {
      severity: m.severity,
      file: inUser ? s.file : ours ? "<engine wrapper>" : (m.file ?? "<unknown>"),
      line: ours && !inUser ? line - userLines : line,
      column: m.column ?? 0,
      message: m.message,
    };
  });
}

async function compile(compiler: string, s: EffectSource, includes: string[]): Promise<Compiled> {
  const userLines = s.source.split("\n").length;
  const module = stemOf(s.file).replace(/[^A-Za-z0-9_]/g, "_");
  const run = (source: string, entry: string, targets: Target[]) =>
    redslangc({ compiler, source, sourceName: s.sourceName, module, includes, targets, entry });
  const fail = (r: RunResult) => {
    let problems = problemsOf(r.messages, s, userLines);
    if (problems.some((p) => p.severity === "error" && p.file === s.file)) {
      problems = problems.filter((p) => p.file !== "<unknown>");
    }
    if (!problems.some((p) => p.severity === "error")) {
      problems.push({ severity: "error", file: s.file, line: 0, column: 0, message: r.diagnostics.trim() });
    }
    return new EffectsError(problems);
  };

  // PASS 1: reflection only; the probe's cbuffer is dropped from the HLSL we discard.
  const probe = await run(`${s.source}\n${probeWrapper(s.struct)}`, "sk_reflect_probe", [TARGETS[0]!]);
  if (!probe.ok) throw fail(probe);
  const { effect, fields } = buildSchema(probe.reflection, s.struct, basename(s.file));

  const header =
    `// GENERATED by the Skathi effect compiler — DO NOT EDIT.\n` +
    `//\n` +
    `//   effect:     ${effect.name}\n` +
    `//   struct:     ${effect.struct}\n` +
    `//   source:     ${s.sourceName}\n` +
    `//   signature:  ${effect.signature}\n` +
    `//   slang:      ${PIN.slang}\n` +
    `//   schema:     ${SCHEMA}\n` +
    `//   src-sha256: ${sha256(s.source)}\n` +
    `//   output:     ${
      effect.backdrop
        ? "premultiplied RGBA, written with blending disabled (it owns its quad)"
        : "premultiplied RGBA, blended One, OneMinusSrcAlpha"
    }\n` +
    `//\n`;

  const real = await run(`${s.source}\n${realWrapper(effect, fields)}`, ENTRY, TARGETS);
  if (!real.ok) throw fail(real);
  const outputs = TARGETS.map((target) => {
    const body = real.code[target.name];
    if (body === undefined) throw new ShaderCompilerError(`redslangc returned no ${target.name} for ${s.file}`);
    target.check(body, s.file);
    return { target: target.name, code: `${header}${target.note()}\n${body}` };
  });

  return {
    file: s.file,
    effect,
    outputs,
    reflection: bindings(real.reflection),
    sourceSha256: sha256(s.source),
    warnings: problemsOf(real.messages, s, userLines).filter((p) => p.severity !== "error"),
  };
}

/// Compiles every source, in parallel; an `EffectsError` lists every effect's problems at once.
export async function compileEffects(
  compiler: string,
  sources: EffectSource[],
  includes: string[] = [],
): Promise<Compiled[]> {
  const results: Array<Compiled | Problem[]> = new Array(sources.length);
  let next = 0;
  const worker = async () => {
    while (next < sources.length) {
      const i = next++;
      const s = sources[i]!;
      try {
        results[i] = await compile(compiler, s, includes);
      } catch (e) {
        if (e instanceof EffectsError) results[i] = e.problems;
        else if (e instanceof SchemaError) results[i] = [schemaProblem(s.file, s.source, e)];
        else throw e;
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(sources.length, availableParallelism()) }, worker));
  const problems = results.flatMap((r) => (Array.isArray(r) ? r : []));
  if (problems.length) throw new EffectsError(problems);
  return results as Compiled[];
}

export interface EffectFile {
  name: string;
  text: string;
}

/// The generated sources and, last, the effects.json that lists them.
export function effectFiles(
  kind: "builtin" | "user",
  compiled: Compiled[],
  sourceOf: (file: string) => string,
): EffectFile[] {
  const manifest = {
    schema: SCHEMA,
    slang: PIN.slang,
    compiler: `redslangc ${PIN.version}`,
    kind,
    preludeSha256: sha256([PRELUDE, ...STD_MODULES.map(([, file]) => file)].map(readText).join("\0")),
    effects: compiled.map((c, index) => ({
      id: index,
      name: c.effect.name,
      struct: c.effect.struct,
      doc: c.effect.doc,
      backdrop: c.effect.backdrop,
      content: c.effect.content,
      history: c.effect.history,
      backdropLinear: c.effect.backdropLinear,
      ...Object.fromEntries(c.outputs.map((o) => [o.target, `${stemOf(c.file)}.${o.target}`])),
      source: sourceOf(c.file),
      sourceSha256: c.sourceSha256,
      generatedSha256: Object.fromEntries(c.outputs.map((o) => [o.target, sha256(o.code)])),
      signature: c.effect.signature,
      backdropRadiusParam: c.effect.backdropRadiusParam,
      backdropReachParam: c.effect.backdropReachParam,
      bleedParam: c.effect.bleedParam,
      animatedParam: c.effect.animatedParam,
      pointerParam: c.effect.pointerParam,
      pointerTimesParam: c.effect.pointerTimesParam,
      qualityParam: c.effect.qualityParam,
      clockSlots: c.effect.clockSlots,
      clockWindow: c.effect.clockWindow,
      params: c.effect.params,
      bindings: c.reflection,
    })),
  };
  return [
    ...compiled.flatMap((c) => c.outputs.map((o) => ({ name: `${stemOf(c.file)}.${o.target}`, text: o.code }))),
    { name: "effects.json", text: `${JSON.stringify(manifest, null, 2)}\n` },
  ];
}

export const appShadersDir = (appRoot: string) => join(appRoot, "shaders");

/// Whether the app has effects of its own: any `shaders/*.slang`.
export function hasAppEffects(appRoot: string): boolean {
  const dir = appShadersDir(appRoot);
  try {
    return statSync(dir).isDirectory() && slangFiles(dir).some((f) => DECLARES_EFFECT.test(readText(f)));
  } catch {
    return false;
  }
}

export interface AppEffects {
  compiled: Compiled[];
  files: EffectFile[];
  warnings: Problem[];
}

/// Compiles `<appRoot>/shaders/*.slang`; the app's shaders directory is an include path too, for shared helpers.
export async function compileAppEffects(appRoot: string, compiler: string): Promise<AppEffects> {
  const dir = appShadersDir(appRoot);
  const posix = (file: string) => relative(appRoot, file).split(/[\\/]/).join("/");
  const { sources, problems } = discover(dir, (file) => `${posix(file)}+wrapper`);
  if (problems.length) throw new EffectsError(problems);
  const compiled = await compileEffects(compiler, sources, [dir]);
  const seen = new Map<string, string>();
  for (const c of compiled) {
    if (BUILTIN_EFFECTS.includes(c.effect.name)) {
      problems.push({
        ...schemaProblemAt(c, `[SkEffect("${c.effect.name}")] shadows a built-in effect; pick another name`),
      });
    }
    const previous = seen.get(c.effect.name);
    if (previous) {
      problems.push(
        schemaProblemAt(
          c,
          `[SkEffect("${c.effect.name}")] is declared in ${basename(previous)} too; names must be unique`,
        ),
      );
    }
    seen.set(c.effect.name, c.file);
  }
  if (problems.length) throw new EffectsError(problems);
  return { compiled, files: effectFiles("user", compiled, posix), warnings: compiled.flatMap((c) => c.warnings) };
}

function schemaProblemAt(c: Compiled, message: string): Problem {
  const lines = readText(c.file).split("\n");
  const at = lines.findIndex((l) => l.includes("SkEffect"));
  return {
    severity: "error",
    file: c.file,
    line: at + 1,
    column: at < 0 ? 0 : lines[at]!.indexOf("SkEffect") + 1,
    message,
  };
}

/// Replaces `dir`'s generated files with `files`, effects.json last: a host reads it to decide what to load.
export function writeEffectFiles(dir: string, files: EffectFile[]): void {
  mkdirSync(dir, { recursive: true });
  const keep = new Set(files.map((f) => f.name));
  for (const name of readdirSync(dir)) {
    if (!keep.has(name) && /\.(hlsl|wgsl|metal)$/.test(name)) rmSync(join(dir, name), { force: true });
  }
  for (const f of files) if (f.name !== "effects.json") writeFileSync(join(dir, f.name), f.text);
  const manifest = files.find((f) => f.name === "effects.json");
  if (manifest) writeFileSync(join(dir, manifest.name), manifest.text);
}
