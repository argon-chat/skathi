// The `--shading` schema as the Effects panel reads it: effects.json text from the host, read
// field by field so a newer manifest with more attributes still shows what this panel knows.

export type ArgKind = "number" | "length" | "color";

const EDITABLE: readonly string[] = ["number", "length", "color"];

export interface EffectParam {
  name: string;
  kind: string;
  slot: number | null;
  required: boolean;
  default: number[];
  range: [number, number] | null;
  /** An author can write it in `--shading`; otherwise the host fills it (or the kind is newer than this panel). */
  editable: boolean;
}

export interface EffectFlag {
  key: string;
  label: string;
  title: string;
}

export interface EffectInfo {
  name: string;
  doc: string;
  signature: string;
  builtin: boolean;
  origin: string;
  params: EffectParam[];
  flags: EffectFlag[];
  /** `false` when the host has the manifest but did not register the effect (e.g. a schema mismatch). */
  loaded: boolean;
}

export interface ManifestEntry {
  origin: string;
  builtin: boolean;
  manifest: unknown;
}

export interface LoadedEffect {
  name: string;
  id: number;
  builtin: boolean;
}

export interface ShadingDeclaration {
  selector: string;
  state: string;
  value: string;
  important?: boolean;
  media?: string;
}

/** `effects node`, and each row of `effects list`. */
export interface ShadingNode {
  id: number;
  tag: string | null;
  classes: string[];
  /** The `--shading` text in force: the override, the inline style or the winning rule. */
  value: string | null;
  source: "override" | "inline" | "rule" | "none";
  effect: string | null;
  /** The base state's resolved parameter slots. */
  params: number[] | null;
  animated: boolean;
  overridden: boolean;
  /** What the app declares, without the inspector's override. */
  authored: string | null;
  authoredSource: "inline" | "rule" | "none";
  inline: string | null;
  states: { state: string; effect: string | null; params: number[] }[];
  declarations: ShadingDeclaration[];
}

export interface EffectsReply {
  manifests: ManifestEntry[];
  loaded: LoadedEffect[];
  nodes: ShadingNode[];
}

export interface SetReply {
  id: number;
  value: string;
  applies: boolean;
  diagnostics: string[];
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown): string => (typeof v === "string" ? v : "");
const finite = (v: unknown): number | null => (typeof v === "number" && Number.isFinite(v) ? v : null);

function readParam(raw: unknown): EffectParam | null {
  if (!isObj(raw)) return null;
  const name = str(raw.name);
  if (!name) return null;
  const kind = str(raw.kind) || "unknown";
  const range = Array.isArray(raw.range) ? raw.range.map(finite) : null;
  const hostFilled = raw.host !== undefined && raw.host !== false && raw.host !== null;
  return {
    name,
    kind,
    slot: finite(raw.slot),
    required: raw.required === true,
    default: Array.isArray(raw.default) ? raw.default.map((d) => finite(d) ?? 0) : [],
    range: range && range.length === 2 && range[0] !== null && range[1] !== null ? [range[0]!, range[1]!] : null,
    editable: EDITABLE.includes(kind) && !hostFilled,
  };
}

const KNOWN_KEYS = new Set([
  "id",
  "name",
  "struct",
  "doc",
  "backdrop",
  "signature",
  "params",
  "bindings",
  "source",
  "sourceSha256",
  "generatedSha256",
  "hlsl",
  "wgsl",
  "metal",
  "backdropRadiusParam",
  "bleedParam",
  "animatedParam",
]);

function paramAt(params: EffectParam[], slot: unknown): string {
  const n = finite(slot);
  return params.find((p) => p.slot === n)?.name ?? `slot ${n}`;
}

function readFlags(raw: Record<string, unknown>, params: EffectParam[]): EffectFlag[] {
  const flags: EffectFlag[] = [];
  if (raw.backdrop === true) {
    const radius =
      finite(raw.backdropRadiusParam) === null ? "" : `, blurred by ${paramAt(params, raw.backdropRadiusParam)}`;
    flags.push({
      key: "backdrop",
      label: "reads backdrop",
      title: `samples what is painted behind the element: a render-target snapshot and blur pyramid per frame${radius}`,
    });
  }
  if (finite(raw.bleedParam) !== null) {
    flags.push({
      key: "bleed",
      label: "bleeds",
      title: `paints outside the element's box by ${paramAt(params, raw.bleedParam)}`,
    });
  }
  if (finite(raw.animatedParam) !== null) {
    flags.push({
      key: "animated",
      label: "animated",
      title: `repaints the window every frame while ${paramAt(params, raw.animatedParam)} > 0`,
    });
  }
  for (const [key, value] of Object.entries(raw)) {
    if (KNOWN_KEYS.has(key)) continue;
    if (value === true || (key.endsWith("Param") && finite(value) !== null)) {
      flags.push({ key, label: key, title: `${key}: ${JSON.stringify(value)}` });
    }
  }
  return flags;
}

export function readEffects(manifests: readonly ManifestEntry[], loaded: readonly LoadedEffect[]): EffectInfo[] {
  const out: EffectInfo[] = [];
  for (const entry of manifests) {
    const effects = isObj(entry.manifest) && Array.isArray(entry.manifest.effects) ? entry.manifest.effects : [];
    for (const raw of effects) {
      if (!isObj(raw)) continue;
      const name = str(raw.name);
      if (!name) continue;
      const params = (Array.isArray(raw.params) ? raw.params : [])
        .map(readParam)
        .filter((p): p is EffectParam => p !== null);
      out.push({
        name,
        doc: str(raw.doc),
        signature: str(raw.signature) || `${name}(…)`,
        builtin: entry.builtin,
        origin: entry.origin,
        params,
        flags: readFlags(raw, params),
        loaded: loaded.some((l) => l.name.toLowerCase() === name.toLowerCase() && l.builtin === entry.builtin),
      });
    }
  }
  return out;
}

export interface Scalar {
  min: number;
  max: number;
  step: number;
  unit: "" | "px";
}

function niceStep(span: number): number {
  if (span <= 2) return 0.01;
  if (span <= 20) return 0.1;
  return 1;
}

/** Slider bounds: the schema's range, else one around the default. */
export function scalarFor(param: EffectParam): Scalar {
  const unit = param.kind === "length" ? "px" : "";
  const d = param.default[0] ?? 0;
  let min: number, max: number;
  if (param.range) {
    [min, max] = param.range;
  } else if (param.kind === "length") {
    min = Math.min(0, d * 2);
    max = Math.max(64, Math.ceil(Math.abs(d) * 2));
  } else if (d >= 0 && d <= 1) {
    min = 0;
    max = 1;
  } else {
    const m = Math.max(1, Math.ceil(Math.abs(d) * 2));
    min = d < 0 ? -m : 0;
    max = m;
  }
  return { min, max, step: niceStep(max - min), unit };
}

export type Control =
  | { type: "scalar"; key: string; param: EffectParam; scalar: Scalar }
  | { type: "color"; key: string; param: EffectParam }
  | { type: "vector"; key: string; name: string; parts: { axis: string; param: EffectParam; scalar: Scalar }[] }
  | { type: "fixed"; key: string; param: EffectParam };

const AXIS = /^(.+)\.([xyzw])$/;

/** One control per parameter; `field.x`/`field.y`/… components of one vector share a control. */
export function controlsFor(effect: EffectInfo): Control[] {
  const out: Control[] = [];
  for (const param of effect.params) {
    if (!param.editable) {
      out.push({ type: "fixed", key: param.name, param });
      continue;
    }
    const axis = AXIS.exec(param.name);
    if (axis && param.kind !== "color") {
      const prev = out[out.length - 1];
      const part = { axis: axis[2]!, param, scalar: scalarFor(param) };
      if (prev && prev.type === "vector" && prev.name === axis[1]) prev.parts.push(part);
      else out.push({ type: "vector", key: axis[1]!, name: axis[1]!, parts: [part] });
      continue;
    }
    out.push(
      param.kind === "color"
        ? { type: "color", key: param.name, param }
        : { type: "scalar", key: param.name, param, scalar: scalarFor(param) },
    );
  }
  return out;
}

export interface Parsed {
  name: string;
  args: string[];
}

/** `frost(0.6, 14px, #7fd4ff28)` → name and top-level arguments; `null` for `none` or anything else. */
export function parseShading(text: string | null | undefined): Parsed | null {
  const m = /^\s*([A-Za-z_][\w-]*)\s*\(([\s\S]*)\)\s*$/.exec(text ?? "");
  if (!m) return null;
  const args: string[] = [];
  let depth = 0,
    cur = "";
  for (const ch of m[2]!) {
    if (ch === "(") depth++;
    if (ch === ")") depth--;
    if (ch === "," && depth === 0) {
      args.push(cur.trim());
      cur = "";
    } else cur += ch;
  }
  if (cur.trim() || args.length) args.push(cur.trim());
  return { name: m[1]!, args };
}

const NUM = /^[+-]?(\d+\.?\d*|\.\d+)(e[+-]?\d+)?/i;

export function argKind(arg: string): ArgKind {
  const n = NUM.exec(arg);
  if (n) {
    const rest = arg.slice(n[0].length);
    if (rest === "" || rest === "%") return "number";
    if (/^[a-z]+$/i.test(rest)) return "length";
  }
  return "color";
}

function argMatches(arg: string, kind: string): boolean {
  const k = argKind(arg);
  if (k === kind) return true;
  return k === "number" && kind === "length" && Number(arg) === 0;
}

/** Which parameter each argument lands on, by the host's type-directed matching (an optional one is skipped when the argument's type differs). */
export function assign(effect: EffectInfo, args: readonly string[]): Map<string, string> {
  const out = new Map<string, string>();
  let next = 0;
  for (const param of effect.params) {
    if (!param.editable) continue;
    const arg = args[next];
    if (arg === undefined) break;
    if (!argMatches(arg, param.kind)) {
      if (!param.required) continue;
      break;
    }
    out.set(param.name, arg);
    next++;
  }
  return out;
}

export interface Draft {
  values: Record<string, number[]>;
  enabled: Record<string, boolean>;
}

function parseScalar(arg: string): number | null {
  const n = NUM.exec(arg);
  if (!n) return null;
  const v = Number(n[0]);
  return arg.endsWith("%") ? v / 100 : v;
}

function parseHex(arg: string): number[] | null {
  const m = /^#([0-9a-f]{3,8})$/i.exec(arg.trim());
  if (!m || ![3, 4, 6, 8].includes(m[1]!.length)) return null;
  let h = m[1]!;
  if (h.length <= 4) h = [...h].map((c) => c + c).join("");
  const out = [0, 2, 4, 6].map((i) => (i < h.length ? parseInt(h.slice(i, i + 2), 16) / 255 : 1));
  return out;
}

/**
 * The editor's starting point for `effect`: the host-resolved slots when the element paints this
 * effect, else what the declaration spells, else the defaults; an optional parameter is on when
 * the declaration supplies it.
 */
export function draftFrom(effect: EffectInfo, text: string | null, resolved: readonly number[] | null): Draft {
  const parsed = parseShading(text);
  const supplied =
    parsed && parsed.name.toLowerCase() === effect.name.toLowerCase() ? assign(effect, parsed.args) : null;
  const values: Record<string, number[]> = {};
  const enabled: Record<string, boolean> = {};
  for (const param of effect.params) {
    const width = param.kind === "color" ? 4 : 1;
    let value = Array.from({ length: width }, (_, i) => param.default[i] ?? (param.kind === "color" ? 1 : 0));
    const arg = supplied?.get(param.name);
    if (resolved && param.slot !== null && resolved.length >= param.slot + width) {
      value = resolved.slice(param.slot, param.slot + width);
    } else if (arg !== undefined) {
      const v = param.kind === "color" ? parseHex(arg) : [parseScalar(arg)];
      if (v && v.every((x) => x !== null)) value = v as number[];
    }
    values[param.name] = value;
    enabled[param.name] = param.required || (supplied ? supplied.has(param.name) : false);
  }
  return { values, enabled };
}

export function fmtNumber(value: number, step = 0.001): string {
  const decimals = Math.max(0, Math.min(6, Math.ceil(-Math.log10(step))));
  const fixed = value.toFixed(decimals);
  const trimmed = fixed.includes(".") ? fixed.replace(/0+$/, "").replace(/\.$/, "") : fixed;
  return trimmed === "-0" ? "0" : trimmed;
}

const byte = (c: number) =>
  Math.round(Math.max(0, Math.min(1, c)) * 255)
    .toString(16)
    .padStart(2, "0");

export function hexColor(rgba: readonly number[], withAlpha = true): string {
  const [r = 0, g = 0, b = 0, a = 1] = rgba;
  const rgb = `#${byte(r)}${byte(g)}${byte(b)}`;
  return withAlpha && a < 1 ? rgb + byte(a) : rgb;
}

function argText(param: EffectParam, value: readonly number[]): string {
  if (param.kind === "color") return hexColor(value);
  const v = fmtNumber(value[0] ?? 0, Math.min(0.001, scalarFor(param).step));
  return param.kind === "length" ? `${v}px` : v;
}

/**
 * The `--shading` text for a draft. A switched-off optional parameter is still written when
 * leaving it out would let the next argument land on it (a `0` matches a `<length>`).
 */
export function cssText(effect: EffectInfo, draft: Draft): string {
  const params = effect.params.filter((p) => p.editable);
  const texts = params.map((p) => argText(p, draft.values[p.name] ?? p.default));
  const on = params.map((p) => p.required || draft.enabled[p.name] === true);
  for (let i = params.length - 1; i >= 0; i--) {
    if (on[i]) continue;
    const next = on.indexOf(true, i + 1);
    if (next >= 0 && argMatches(texts[next]!, params[i]!.kind)) on[i] = true;
  }
  const args = texts.filter((_, i) => on[i]);
  return `${effect.name}(${args.join(", ")})`;
}
