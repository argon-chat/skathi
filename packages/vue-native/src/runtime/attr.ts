import type { MirrorElement } from "./types";
// `el.attributes` is a live `NamedNodeMap` over `attrs`; each attribute is one `Attr` while the element has it
// (removing it detaches the Attr, which keeps its value). Names are kept as written; namespaces come from the
// prefix (`xlink:`, `xml:`, `xmlns`).

const XLINK_NS = "http://www.w3.org/1999/xlink";
const XML_NS = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS = "http://www.w3.org/2000/xmlns/";

export function attrParts(name: string): { ns: string | null; prefix: string | null; local: string } {
  if (name === "xmlns") return { ns: XMLNS_NS, prefix: null, local: name };
  const i = name.indexOf(":");
  if (i > 0) {
    const prefix = name.slice(0, i);
    const ns = prefix === "xlink" ? XLINK_NS : prefix === "xml" ? XML_NS : prefix === "xmlns" ? XMLNS_NS : null;
    if (ns) return { ns, prefix, local: name.slice(i + 1) };
  }
  return { ns: null, prefix: null, local: name };
}

export class AttrImpl {
  _el: MirrorElement | null = null;
  _name: string;
  _value: string;
  constructor(name: string, value: string) {
    this._name = name;
    this._value = value;
  }
  get name(): string {
    return this._name;
  }
  get nodeName(): string {
    return this._name;
  }
  get localName(): string {
    return attrParts(this._name).local;
  }
  get prefix(): string | null {
    return attrParts(this._name).prefix;
  }
  get namespaceURI(): string | null {
    return attrParts(this._name).ns;
  }
  get ownerElement(): MirrorElement | null {
    return this._el;
  }
  get value(): string {
    if (this._el) this._value = this._el.getAttribute(this._name) ?? this._value;
    return this._value;
  }
  set value(v: string) {
    const s = String(v);
    this._value = s;
    if (this._el) this._el.setAttribute(this._name, s);
  }
  get nodeValue(): string {
    return this.value;
  }
  set nodeValue(v: string) {
    this.value = v == null ? "" : v;
  }
  get textContent(): string {
    return this.value;
  }
  set textContent(v: string) {
    this.value = v == null ? "" : v;
  }
  get nodeType(): number {
    return 2;
  }
  get specified(): boolean {
    return true;
  }
  get isConnected(): boolean {
    return false;
  }
  get parentNode(): null {
    return null;
  }
  get ownerDocument(): unknown {
    return (globalThis as { document?: unknown }).document ?? null;
  }
  cloneNode(): AttrImpl {
    return new AttrImpl(this._name, this.value);
  }
}
Object.defineProperty(AttrImpl, "name", { value: "Attr" });
for (const [k, v] of [
  ["ATTRIBUTE_NODE", 2],
  ["ELEMENT_NODE", 1],
  ["TEXT_NODE", 3],
  ["COMMENT_NODE", 8],
] as const) {
  Object.defineProperty(AttrImpl.prototype, k, { value: v, enumerable: true });
}
