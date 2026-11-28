/// <reference types="vite/client" />
import { parser } from "@lezer/rust";
import catalog from "../../../../crates/sk-css/src/catalog.rs?raw";
import prefixes from "../../../../crates/sk-css/src/prefix.rs?raw";

export const catalogPath = "crates/sk-css/src/catalog.rs";
type Node = ReturnType<typeof parser.parse>["topNode"];
type NativeStatus = "supported" | "partial" | "ignored" | "unsupported" | "dropped";
export interface NativeProperty {
  name: string;
  status: NativeStatus;
  applied: string[];
  refused: string[];
  note: string;
  webkit: boolean;
}

function items(list: Node) {
  const children: Node[] = [];
  for (let node = list.firstChild; node; node = node.nextSibling) {
    if (!["[", "]", "(", ")", ",", "LineComment", "BlockComment"].includes(node.name)) children.push(node);
  }
  return children;
}

function table(source: string) {
  const text = (node: Node) => source.slice(node.from, node.to);
  const constants = new Map<string, Node>();
  parser.parse(source).iterate({
    enter(node) {
      if (node.name === "ConstItem") {
        const name = node.node.getChild("BoundIdentifier");
        const initializer = node.node.getChild("=")?.nextSibling;
        if (name && initializer) constants.set(text(name), initializer);
        return false;
      }
      return node.name === "SourceFile" || node.name === "AttributeItem";
    },
  });
  function get(name: string) {
    const node = constants.get(name);
    if (!node) throw new Error(`Missing sk-css constant ${name}`);
    return node;
  }
  function resolve(node: Node): Node {
    return node.name === "Identifier" ? get(text(node)) : node;
  }
  function list(node: Node) {
    const array = resolve(node).getChild("ArrayExpression");
    if (!array) throw new Error(`Expected a Rust slice in the CSS catalog: ${text(node)}`);
    return items(array);
  }
  function string(node: Node) {
    const value = resolve(node);
    const source = text(value);
    if (value.name === "RawString") return source.slice(source.indexOf('"') + 1, source.lastIndexOf('"'));
    if (value.name !== "String") throw new Error(`Expected a CSS catalog string: ${text(value)}`);
    const escapes: Record<string, string> = { n: "\n", r: "\r", t: "\t", "0": "\0", "'": "'", '"': '"', "\\": "\\" };
    return source.slice(1, -1).replace(/\\(u\{[\da-f_]+\}|x[\da-f]{2}|\r?\n\s*|.)/gi, (_, escape: string) => {
      if (escape in escapes) return escapes[escape]!;
      if (escape.includes("\n")) return "";
      if (escape.startsWith("u{")) return String.fromCodePoint(parseInt(escape.slice(2, -1).replaceAll("_", ""), 16));
      if (escape.startsWith("x")) return String.fromCharCode(parseInt(escape.slice(1), 16));
      throw new Error(`Unknown Rust string escape in ${source}`);
    });
  }
  return { text, get, string, list };
}

const prefixTable = table(prefixes);
export const webkitNames = prefixTable.list(prefixTable.get("WEBKIT_APPLIED")).map(prefixTable.string);
const webkit = new Set(webkitNames);
const data = table(catalog);
const statuses = new Map<string, NativeStatus>([
  ["Supported", "supported"],
  ["Partial", "partial"],
  ["Ignored", "ignored"],
  ["Unsupported", "unsupported"],
  ["Dropped", "dropped"],
]);

export const nativeProperties = new Map<string, NativeProperty>();
for (const row of data.list(data.get("CATALOG"))) {
  const call = row.getChild("ArgList");
  const [name, status, applied, refused, note, ...rest] = call ? items(call) : [];
  if (
    row.firstChild?.name !== "Identifier" ||
    data.text(row.firstChild) !== "e" ||
    !name ||
    !status ||
    !applied ||
    !refused ||
    !note ||
    rest.length
  )
    throw new Error(`Unrecognized sk-css catalog entry: ${data.text(row)}`);
  const key = data.string(name);
  const support = statuses.get(data.text(status));
  if (!support || nativeProperties.has(key)) throw new Error(`Invalid CSS catalog entry: ${key}`);
  nativeProperties.set(key, {
    name: key,
    status: support,
    applied: data.list(applied).map(data.string),
    refused: data.list(refused).map(data.string),
    note: data.string(note),
    webkit: webkit.has(key),
  });
}
