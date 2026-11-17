import schemaJson from "../../../packages/runtime/sk.config.schema.json";
import { groups, notes, type ConfigNote } from "../data/config/notes";
import { entries as windowEntries, type WindowEntry } from "../data/window/entries";
import {
  categoryId,
  entryMarkdownUrl,
  entryUrl,
  freshNote,
  newLabel,
  platforms,
  platformsDiffer,
  sinceOn,
  statuses,
  type Release,
  type Status,
} from "./reference";
import { windowReference } from "./window";

interface Schema {
  description?: string;
  type?: string;
  enum?: readonly unknown[];
  const?: unknown;
  default?: unknown;
  properties?: Readonly<Record<string, Schema>>;
  patternProperties?: Readonly<Record<string, Schema>>;
  additionalProperties?: boolean | Schema;
  propertyNames?: Schema;
  required?: readonly string[];
  items?: Schema;
  oneOf?: readonly Schema[];
  not?: Schema;
  pattern?: string;
  minLength?: number;
  minimum?: number;
  exclusiveMinimum?: number;
  minItems?: number;
  uniqueItems?: boolean;
  $ref?: string;
  definitions?: Readonly<Record<string, Schema>>;
}

export interface ConfigValue {
  value: string;
  note?: string;
}

export interface ConfigOption {
  /** "window.titleBarOverlay.color", "packs[].name". */
  path: string;
  type: string;
  description: string;
  default?: string;
  values: ConfigValue[];
  constraints: string[];
  required: boolean;
  /** Per-OS support, only when it differs between them. */
  support?: { label: string; status: Status; since?: Release }[];
  /** What arrived in the current release, from the matching Window API entry. */
  fresh?: string;
  platforms?: string;
  notes: string[];
  limitations: string[];
  example?: string;
  more?: { name: string; slug: string };
  children: ConfigOption[];
}

export interface ConfigSection {
  title: string;
  id: string;
  intro: string;
  options: ConfigOption[];
}

export const configPage = { id: "sk-config", schema: "packages/runtime/sk.config.schema.json" } as const;

const root = schemaJson as Schema;

function deref(node: Schema): Schema {
  if (!node.$ref) return node;
  const name = node.$ref.replace("#/definitions/", "");
  const target = root.definitions?.[name];
  if (!target) throw new Error(`${configPage.schema}: unresolved ${node.$ref}.`);
  return node.description ? { ...target, description: node.description } : target;
}

/** The object whose properties nest under this key: itself, its object branch, or its array items. */
function objectOf(node: Schema): Schema | undefined {
  const n = deref(node);
  if (n.properties) return n;
  const branch = n.oneOf?.find((b) => deref(b).properties);
  if (branch) return deref(branch);
  return n.items && deref(n.items).properties ? deref(n.items) : undefined;
}

function typeOf(node: Schema): string {
  const n = deref(node);
  if (n.oneOf) return n.oneOf.map(typeOf).join(" | ");
  if ("const" in n) return JSON.stringify(n.const);
  if (n.enum) return n.enum.map((v) => JSON.stringify(v)).join(" | ");
  if (n.type === "array") {
    const item = n.items ? typeOf(n.items) : "unknown";
    return item.includes(" | ") ? `(${item})[]` : `${item}[]`;
  }
  if (n.type === "object") {
    if (n.properties) return "object";
    if (typeof n.additionalProperties === "object") return `Record<string, ${typeOf(n.additionalProperties)}>`;
    return "Record<string, unknown>";
  }
  return n.type ?? "unknown";
}

function constraintsOf(node: Schema, prefix = ""): string[] {
  const n = deref(node);
  const out: string[] = [];
  const say = (text: string) => out.push(`${prefix}${text}`);
  if (n.pattern === "\\S") say("not blank");
  else if (n.minLength !== undefined) say(n.minLength === 1 ? "not empty" : `at least ${n.minLength} characters`);
  else if (n.pattern) say(`matches \`/${n.pattern}/\``);
  if (n.minimum !== undefined) say(`>= ${n.minimum}`);
  if (n.exclusiveMinimum !== undefined) say(`> ${n.exclusiveMinimum}`);
  if (n.not && "const" in n.not) say(`not ${JSON.stringify(n.not.const)}`);
  if (n.minItems !== undefined) say(n.minItems === 1 ? "at least one item" : `at least ${n.minItems} items`);
  if (n.uniqueItems) say("no duplicates");
  if (n.propertyNames?.pattern) say(`keys match \`/${n.propertyNames.pattern}/\``);
  if (n.additionalProperties === false) say("unknown keys are errors");
  if (n.items) out.push(...constraintsOf(n.items, "each item: "));
  if (typeof n.additionalProperties === "object") out.push(...constraintsOf(n.additionalProperties, "each value: "));
  for (const branch of n.oneOf ?? []) {
    const b = deref(branch);
    if (b.properties && b.required?.length) say(`an object requires ${b.required.map((k) => `\`${k}\``).join(", ")}`);
    if (b.properties && b.additionalProperties === false) say("an object with unknown keys is an error");
    if (!b.properties) out.push(...constraintsOf(b, prefix));
  }
  return out;
}

export function constraintText(constraints: readonly string[]): string {
  const text = constraints.join("; ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function enumOf(node: Schema): readonly unknown[] | undefined {
  const n = deref(node);
  return n.enum ?? (n.items ? deref(n.items).enum : undefined);
}

function validate(value: unknown, node: Schema, where: string): string[] {
  const n = deref(node);
  if (n.oneOf) {
    return n.oneOf.some((b) => validate(value, b, where).length === 0)
      ? []
      : [`${where}: ${JSON.stringify(value)} is none of ${typeOf(n)}`];
  }
  const errors: string[] = [];
  const fail = (why: string) => errors.push(`${where}: ${why}`);
  if ("const" in n && value !== n.const) fail(`must be ${JSON.stringify(n.const)}`);
  if (n.enum && !n.enum.includes(value)) fail(`${JSON.stringify(value)} is not one of ${typeOf(n)}`);
  if (n.not && validate(value, n.not, where).length === 0) fail(`${JSON.stringify(value)} is not allowed`);
  switch (n.type) {
    case "string":
      if (typeof value !== "string") fail("must be a string");
      else {
        if (n.minLength !== undefined && value.length < n.minLength) fail("is too short");
        if (n.pattern && !new RegExp(n.pattern).test(value)) fail(`does not match ${n.pattern}`);
      }
      break;
    case "number":
      if (typeof value !== "number") fail("must be a number");
      else if (
        (n.minimum !== undefined && value < n.minimum) ||
        (n.exclusiveMinimum !== undefined && value <= n.exclusiveMinimum)
      )
        fail("is out of range");
      break;
    case "boolean":
      if (typeof value !== "boolean") fail("must be a boolean");
      break;
    case "array":
      if (!Array.isArray(value)) fail("must be an array");
      else {
        if (n.minItems !== undefined && value.length < n.minItems) fail("has too few items");
        if (n.items) {
          const items = n.items;
          value.forEach((item, i) => errors.push(...validate(item, items, `${where}[${i}]`)));
        }
      }
      break;
    case "object": {
      if (!value || typeof value !== "object" || Array.isArray(value)) {
        fail("must be an object");
        break;
      }
      const record = value as Record<string, unknown>;
      for (const key of n.required ?? []) if (!(key in record)) fail(`needs ${key}`);
      for (const [key, v] of Object.entries(record)) {
        const prop = n.properties?.[key];
        const patterned = Object.entries(n.patternProperties ?? {}).find(([p]) => new RegExp(p).test(key));
        if (n.propertyNames?.pattern && !new RegExp(n.propertyNames.pattern).test(key)) fail(`bad key ${key}`);
        if (prop) errors.push(...validate(v, prop, `${where}.${key}`));
        else if (patterned) errors.push(...validate(v, patterned[1], `${where}.${key}`));
        else if (n.additionalProperties === false) fail(`unknown key ${key}`);
        else if (typeof n.additionalProperties === "object")
          errors.push(...validate(v, n.additionalProperties, `${where}.${key}`));
      }
      break;
    }
  }
  return errors;
}

/** One-line JSON with spaces, as people write it. */
function inlineJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(inlineJson).join(", ")}]`;
  if (value && typeof value === "object") {
    const parts = Object.entries(value).map(([k, v]) => `${JSON.stringify(k)}: ${inlineJson(v)}`);
    return parts.length ? `{ ${parts.join(", ")} }` : "{}";
  }
  return JSON.stringify(value);
}

/** Pretty JSON that keeps a value on one line when it fits in `width` columns; the outermost object never does. */
function formatJson(value: unknown, indent = 0, used = -1, width = 88): string {
  const one = inlineJson(value);
  if (used >= 0 && used + one.length <= width) return one;
  const pad = " ".repeat(indent + 2);
  const close = " ".repeat(indent);
  if (Array.isArray(value))
    return `[\n${value.map((v) => pad + formatJson(v, indent + 2, pad.length)).join(",\n")}\n${close}]`;
  if (value && typeof value === "object")
    return `{\n${Object.entries(value)
      .map(([k, v]) => {
        const head = `${pad}${JSON.stringify(k)}: `;
        return head + formatJson(v, indent + 2, head.length);
      })
      .join(",\n")}\n${close}}`;
  return one;
}

const segments = (path: string): string[] => path.replace(/\[\]/g, ".[]").split(".");
const joinPath = (parts: readonly string[]): string => parts.join(".").replace(/\.\[\]/g, "[]");

function exampleValue(path: string, node: Schema): unknown {
  const note = notes[path];
  if (note && "example" in note) return note.example;
  const n = deref(node);
  if (n.enum?.length) return n.enum.find((v) => v !== n.default) ?? n.enum[0];
  if (n.type === "boolean") return n.default === undefined ? true : !n.default;
  return undefined;
}

/** The key's example value nested in a whole sk.config.json, with the required keys of each enclosing object. */
function exampleDocument(path: string, value: unknown, nodes: ReadonlyMap<string, Schema>): unknown {
  let current = value;
  const parts = segments(path);
  for (let i = parts.length - 1; i >= 0; i--) {
    const part = parts[i] ?? "";
    if (part === "[]") {
      current = [current];
      continue;
    }
    const outer = parts.slice(0, i);
    const ownerPath = joinPath(outer.at(-1) === "[]" ? outer.slice(0, -1) : outer);
    const owner = i === 0 ? root : objectOf(nodes.get(ownerPath) ?? {});
    const siblings = (owner?.required ?? [])
      .filter((key) => key !== part)
      .map((key) => {
        const sibling = joinPath([...outer, key]);
        return [key, exampleValue(sibling, nodes.get(sibling) ?? {})] as const;
      });
    current = Object.fromEntries([...siblings, [part, current]]);
  }
  return current;
}

const optionEntries = new Map(
  windowEntries.filter((entry) => entry.kind === "option").map((entry) => [entry.name, entry] as const),
);

function build(): ConfigSection[] {
  const nodes = new Map<string, Schema>();
  const walk = (node: Schema, prefix: string) => {
    const object = objectOf(node);
    for (const [key, child] of Object.entries(object?.properties ?? {})) {
      const path = `${prefix}${key}`;
      nodes.set(path, child);
      walk(child, `${path}${deref(child).type === "array" ? "[]" : ""}.`);
    }
  };
  walk(root, "");

  for (const path of Object.keys(notes))
    if (!nodes.has(path)) throw new Error(`docs/src/data/config/notes.ts: ${path} is not in ${configPage.schema}.`);

  const option = (path: string, node: Schema, required: boolean): ConfigOption => {
    const n = deref(node);
    const note: ConfigNote = notes[path] ?? {};
    const entry: WindowEntry | undefined = optionEntries.get(path);
    if (n.default !== undefined && note.default !== undefined)
      throw new Error(`docs/src/data/config/notes.ts: ${path} has a default in the schema; drop the one in the notes.`);
    const fallbackDefault = entry?.params?.find((p) => p.name === path.split(".").at(-1))?.default;
    const values = (enumOf(n) ?? []).map((raw): ConfigValue => {
      const value = String(raw);
      const text = note.values?.[value] ?? entry?.values?.find((v) => v.value === value)?.note;
      return text === undefined ? { value } : { value, note: text };
    });
    const example = exampleValue(path, n);
    let exampleText: string | undefined;
    if (example !== undefined) {
      const document = exampleDocument(path, example, nodes);
      const errors = validate(document, root, "sk.config.json");
      if (errors.length)
        throw new Error(`The example of ${path} does not match ${configPage.schema}: ${errors.join("; ")}`);
      exampleText = formatJson(document);
    }
    const object = objectOf(n);
    const childPrefix = `${path}${n.type === "array" ? "[]" : ""}.`;
    const result: ConfigOption = {
      path,
      type: typeOf(n),
      description: n.description ?? "",
      values,
      constraints: constraintsOf(n),
      required,
      notes: [...(note.notes ?? [])],
      limitations: [...(entry?.limitations ?? [])],
      children: Object.entries(object?.properties ?? {}).map(([key, child]) =>
        option(`${childPrefix}${key}`, child, object?.required?.includes(key) ?? false),
      ),
    };
    const dflt = n.default !== undefined ? `\`${JSON.stringify(n.default)}\`` : (note.default ?? fallbackDefault);
    if (dflt !== undefined) result.default = dflt;
    if (note.platforms) result.platforms = note.platforms;
    if (exampleText !== undefined) result.example = exampleText;
    if (entry) {
      result.more = { name: entry.name, slug: entry.slug };
      if (platformsDiffer(entry) || entry.platformSince)
        result.support = platforms.map((p) => {
          const since = entry.platformSince ? sinceOn(entry, p.id) : undefined;
          return { label: p.label, status: entry.platforms[p.id], ...(since ? { since } : {}) };
        });
      const fresh = freshNote(entry);
      if (fresh) result.fresh = fresh;
    }
    return result;
  };

  const top = Object.entries(root.properties ?? {});
  const placed = new Set<string>();
  const sections = groups.map((group): ConfigSection => {
    const options = group.keys.map((key) => {
      const node = root.properties?.[key];
      if (!node)
        throw new Error(`docs/src/data/config/notes.ts: group ${group.title} names ${key}, not in the schema.`);
      placed.add(key);
      return option(key, node, false);
    });
    return { title: group.title, id: categoryId(group.title), intro: group.intro, options };
  });
  const rest = top.filter(([key]) => !placed.has(key));
  if (rest.length)
    sections.push({
      title: "Other keys",
      id: "other-keys",
      intro: "",
      options: rest.map(([key, node]) => option(key, node, false)),
    });
  return sections;
}

export const configSections: readonly ConfigSection[] = build();

export function optionId(path: string): string {
  return path.replace(/\[\]/g, "");
}

export function windowApiUrl(slug: string): string {
  return entryUrl(windowReference, slug);
}

function optionMarkdown(o: ConfigOption, depth: number, site: URL): string[] {
  const lines = [`${"#".repeat(depth)} \`${o.path}\``, ""];
  if (o.description) lines.push(o.description, "");
  lines.push(`- Type: \`${o.type}\`${o.required ? " (required)" : ""}`);
  if (o.default) lines.push(`- Default: ${o.default}`);
  if (o.constraints.length) lines.push(`- Constraints: ${constraintText(o.constraints)}`);
  if (o.support)
    lines.push(
      `- Platforms: ${o.support.map((s) => `${s.label}: ${statuses[s.status].label.toLowerCase()}${s.since ? ` since ${s.since}` : ""}`).join(", ")}`,
    );
  if (o.fresh) lines.push(`- ${newLabel}: ${o.fresh}.`);
  if (o.platforms) lines.push(`- ${o.support ? "Platform notes" : "Platforms"}: ${o.platforms}`);
  if (o.values.length) {
    lines.push("- Allowed values:");
    for (const v of o.values) lines.push(`  - \`${JSON.stringify(v.value)}\`${v.note ? `: ${v.note}` : ""}`);
  }
  for (const note of [...o.notes, ...o.limitations]) lines.push(`- ${note}`);
  if (o.more)
    lines.push(
      `- Details: [${o.more.name} in the Window API](${new URL(entryMarkdownUrl(windowReference, o.more.slug), site).href})`,
    );
  if (o.example) lines.push("", "```json", o.example, "```");
  lines.push("");
  for (const child of o.children) lines.push(...optionMarkdown(child, Math.min(depth + 1, 6), site));
  return lines;
}

export function configMarkdown(site: URL): string {
  return configSections
    .map((section) =>
      [
        `## ${section.title}`,
        "",
        ...(section.intro ? [section.intro, ""] : []),
        ...section.options.flatMap((o) => optionMarkdown(o, 3, site)),
      ]
        .join("\n")
        .trim(),
    )
    .join("\n\n");
}

export const configHeadings = configSections.flatMap((section) => [
  { depth: 2, slug: section.id, text: section.title },
  ...section.options.map((o) => ({ depth: 3, slug: optionId(o.path), text: o.path })),
]);
