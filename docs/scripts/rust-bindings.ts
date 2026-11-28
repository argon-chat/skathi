import { parser } from "@lezer/rust";
import { existsSync, globSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { workspaceRoot } from "./workspace";

type Node = ReturnType<typeof parser.parse>["topNode"];

interface NativeMember {
  name: string;
  kind: "method" | "getter" | "accessor";
  /** The JavaScript Function.length set by the host. */
  arity?: number;
}

export interface NativeBinding {
  name: string;
  source: string;
  members: NativeMember[];
  documentation?: string;
}

function rustdoc(item: Node, source: string) {
  const lines: string[] = [];
  let node: Node | null = item;
  while (node) {
    if (!node.prevSibling && node.parent?.name === "AttributeItem") {
      node = node.parent;
      continue;
    }
    node = node.prevSibling;
    if (!node) break;
    if (node.name === "Attribute") continue;
    const line = source.slice(node.from, node.to);
    if (node.name !== "LineComment" || !/^\/\/\/(?!\/)/.test(line)) break;
    lines.push(line.slice(3).replace(/^ /, ""));
  }
  return lines.reverse().join("\n").trim();
}

/** Extract literal prototype registrations. Computed members and calls that extend a prototype later
 * are deliberately omitted. The caller joins these records to the SDK's public declarations. */
export function readClassSpecs(source: string, file: string): NativeBinding[] {
  const tree = parser.parse(source);
  const constants = new Map<string, Node>();
  const argumentsToRegister: Node[] = [];
  const documentation = new Map<number, string>();
  const text = (node: Node) => source.slice(node.from, node.to);
  tree.iterate({
    enter(ref) {
      if (ref.name === "AttributeItem") {
        const attrs = ref.node.getChildren("Attribute");
        if (attrs.some((attr) => /^#\[\s*(?:test\s*\]|cfg\s*\(\s*test\s*\)\s*\])/.test(text(attr)))) return false;
      }
      if (ref.name === "ConstItem") {
        const owner = ref.node.parent?.name === "AttributeItem" ? ref.node.parent.parent : ref.node.parent;
        if (owner?.name !== "SourceFile") return;
        const name = ref.node.getChild("BoundIdentifier");
        const value = ref.node.getChild("=")?.nextSibling;
        if (name && value) {
          constants.set(text(name), value);
          const note = rustdoc(ref.node, source);
          if (note) documentation.set(value.from, note);
        }
      }
      if (ref.name !== "CallExpression") return;
      const node = ref.node;
      const callee = node.firstChild && text(node.firstChild).split(/::|\./).at(-1);
      if (callee !== "define_class" && callee !== "register" && callee !== "class") return;
      for (let arg = node.getChild("ArgList")?.firstChild; arg; arg = arg.nextSibling) {
        const value = arg.name === "ReferenceExpression" ? arg.lastChild : arg;
        if (value && ["StructExpression", "Identifier", "ScopedIdentifier"].includes(value.name))
          argumentsToRegister.push(value);
      }
      return;
    },
  });
  function resolve(node: Node | null | undefined): Node | undefined {
    if (!node) return;
    return node.name === "Identifier" ? constants.get(text(node)) : node;
  }
  function literal(node: Node | null | undefined): string | undefined {
    const value = resolve(node);
    if (value?.name !== "String") return;
    const match = /^"([a-zA-Z_$][\w$]*)"$/.exec(text(value));
    return match?.[1];
  }
  const bindings: NativeBinding[] = [];
  const seen = new Set<number>();
  for (const argument of argumentsToRegister) {
    const spec =
      argument.name === "ScopedIdentifier" && text(argument).startsWith("self::")
        ? constants.get(text(argument).slice(6))
        : resolve(argument);
    if (
      spec?.name !== "StructExpression" ||
      !spec.firstChild ||
      text(spec.firstChild).split("::").at(-1) !== "ClassSpec" ||
      seen.has(spec.from)
    )
      continue;
    seen.add(spec.from);
    const fields = new Map<string, Node>();
    for (const field of spec.getChild("FieldInitializerList")?.getChildren("FieldInitializer") ?? []) {
      if (field.firstChild && field.lastChild) fields.set(text(field.firstChild), field.lastChild);
    }
    const name = literal(fields.get("name"));
    if (!name) continue;
    const members: NativeMember[] = [];
    for (const [field, kind] of [
      ["methods", "method"],
      ["getters", "getter"],
      ["accessors", "accessor"],
    ] as const) {
      const value = resolve(fields.get(field));
      const array = value?.name === "ArrayExpression" ? value : value?.getChild("ArrayExpression");
      for (const tuple of array?.getChildren("TupleExpression") ?? []) {
        const first = tuple.getChild("(")?.nextSibling;
        const member = literal(first);
        if (!member) continue;
        const arity = kind === "method" ? tuple.getChild("Integer") : null;
        members.push({
          name: member,
          kind,
          ...(arity && /^\d+$/.test(text(arity)) ? { arity: Number(text(arity)) } : {}),
        });
      }
    }
    const note = documentation.get(spec.from);
    bindings.push({ name, source: file, members, ...(note && { documentation: note }) });
  }
  return bindings;
}

function loadBindings(): Map<string, NativeBinding | undefined> {
  if (!existsSync(join(workspaceRoot, "vendor/redjs/redjs/src/webapi/util.rs"))) {
    throw new Error("The web API reference needs redjs. Run git submodule update --init vendor/redjs in the SDK.");
  }
  const files = globSync(["crates/sk-*/src/**/*.rs", "vendor/redjs/redjs/src/webapi/**/*.rs"], {
    cwd: workspaceRoot,
    exclude: ["**/tests.rs", "**/tests/**"],
  }).sort();
  const bindings = new Map<string, NativeBinding | undefined>();
  for (const path of files) {
    const file = path.replaceAll("\\", "/");
    const source = readFileSync(join(workspaceRoot, path), "utf8");
    if (!source.includes("ClassSpec")) continue;
    const module = /^crates\/sk-app\/src\/stdlib\/([^/.]+)/.exec(file)?.[1];
    const scope = module ? `skathi:${module}` : "web";
    for (const binding of readClassSpecs(source, file)) {
      const key = `${scope}/${binding.name}`;
      // Conditional implementations can register a different prototype under the same name.
      bindings.set(key, bindings.has(key) ? undefined : binding);
    }
  }
  return bindings;
}

let bindings: ReturnType<typeof loadBindings> | undefined;

export function rustBinding(scope: string, name: string): NativeBinding | undefined {
  bindings ??= loadBindings();
  return bindings.get(`${scope}/${name}`);
}
