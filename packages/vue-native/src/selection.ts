// Text selection: `getSelection()`, `Selection`, `Range`, `document.createRange()` (the JS half of
// crates/vn-tree/src/selection.rs).
// The selection lives in the host (drags, double click, Shift+click and Ctrl+A change it without a JS round trip);
// `Selection` reads it on every access through `__vn_selection_get` and writes through `__vn_selection_set`. A boundary
// in an element's own text (no Text child in the mirror) is handed out as a real Text node by `materialize`.
// `Range` is a plain JS object over mirror nodes but STATIC (DOM mutations do not move it), and its rects are element
// boxes, not glyph boxes. `selectstart` and `selectionchange` are host events dispatched by runtime.ts. This module
// reads no ambient names: everything comes in through `env`, and without `env.host` nothing installs.

export interface SelectionHost {
  get(): string;
  set(anchor: number, anchorOffset: number, focus: number, focusOffset: number): boolean;
  text(): string;
}

export interface SelNode {
  __vnId: number;
  kind: number;
  parentNode: SelNode | null;
  childNodes: SelNode[];
  text?: string;
  ownText?: string;
}

export interface SelectionEnv {
  host: SelectionHost | null;
  nodeById(id: number): SelNode | undefined;
  isNode(x: unknown): x is SelNode;
  root(): SelNode | null;
  materialize(el: SelNode): SelNode | null;
  rect(el: SelNode): DOMRectLike;
  makeRect(x: number, y: number, w: number, h: number): DOMRectLike;
  createText(data: string): SelNode;
  createFragment(): SelNode;
  warn(key: string, msg: string): void;
  error(method: string, iface: string, name: string, msg: string): Error;
  defineGlobal(name: string, value: unknown): void;
  defineDoc(name: string, value: unknown): void;
}

export interface DOMRectLike {
  x: number;
  y: number;
  width: number;
  height: number;
  top: number;
  left: number;
  right: number;
  bottom: number;
}

type Mutable = SelNode & {
  data: string;
  insertBefore(node: unknown, ref: unknown): unknown;
  appendChild(node: unknown): unknown;
  removeChild(node: unknown): unknown;
};

type Point = [SelNode, number];

export function installSelection(env: SelectionEnv): void {
  const host = env.host;
  if (!host) return;

  const isText = (n: SelNode) => n.kind === 2 || n.kind === 3;
  const indexOf = (n: SelNode) => (n.parentNode ? n.parentNode.childNodes.indexOf(n) : 0);
  const real = (n: SelNode): SelNode => {
    if (n.kind === 1 && n.ownText && !n.childNodes.length) env.materialize(n);
    return n;
  };
  const lengthOf = (n: SelNode) => (isText(n) ? (n.text ?? "").length : real(n).childNodes.length);
  const rootOf = (n: SelNode) => {
    let r = n;
    while (r.parentNode) r = r.parentNode;
    return r;
  };
  const inDocument = (n: SelNode) => {
    const root = env.root();
    return !!root && rootOf(n) === root;
  };
  const holds = (a: SelNode, b: SelNode | null) => {
    for (let n = b; n; n = n.parentNode) if (n === a) return true;
    return false;
  };
  const compare = (an: SelNode, ao: number, bn: SelNode, bo: number): number => {
    if (an === bn) return Math.sign(ao - bo);
    if (holds(an, bn)) {
      let c = bn;
      while (c.parentNode !== an) c = c.parentNode!;
      return ao <= indexOf(c) ? -1 : 1;
    }
    if (holds(bn, an)) return -compare(bn, bo, an, ao);
    const chain = (n: SelNode) => {
      const out: SelNode[] = [];
      for (let x: SelNode | null = n; x; x = x.parentNode) out.unshift(x);
      return out;
    };
    const ca = chain(an);
    const cb = chain(bn);
    if (ca[0] !== cb[0]) return -1;
    let i = 0;
    while (ca[i] === cb[i]) i++;
    return indexOf(ca[i]!) < indexOf(cb[i]!) ? -1 : 1;
  };
  const point = (node: unknown, offset: unknown, method: string, iface: string): Point => {
    if (!env.isNode(node)) {
      throw new TypeError(`Failed to execute '${method}' on '${iface}': parameter 1 is not of type 'Node'.`);
    }
    const n = real(node);
    const off = Number(offset) >>> 0;
    if (off > lengthOf(n)) {
      throw env.error(method, iface, "IndexSizeError", `There is no child at offset ${off}.`);
    }
    return [n, off];
  };
  const parentPoint = (node: unknown, after: boolean, method: string): Point => {
    if (!env.isNode(node)) {
      throw new TypeError(`Failed to execute '${method}' on 'Range': parameter 1 is not of type 'Node'.`);
    }
    const p = node.parentNode;
    if (!p) throw env.error(method, "Range", "InvalidNodeTypeError", "the given Node has no parent.");
    return [p, indexOf(node) + (after ? 1 : 0)];
  };

  const owners = new WeakMap<RangeImpl, SelectionImpl>();

  class RangeImpl {
    static readonly START_TO_START = 0;
    static readonly START_TO_END = 1;
    static readonly END_TO_END = 2;
    static readonly END_TO_START = 3;
    declare readonly START_TO_START: 0;
    declare readonly START_TO_END: 1;
    declare readonly END_TO_END: 2;
    declare readonly END_TO_START: 3;
    /** @internal */ _sc: SelNode;
    /** @internal */ _so = 0;
    /** @internal */ _ec: SelNode;
    /** @internal */ _eo = 0;

    constructor() {
      const root = env.root();
      // `new Range()` is collapsed at (document, 0); the app root stands for the document here.
      this._sc = this._ec = root ?? ({ __vnId: 0, kind: 9, parentNode: null, childNodes: [] } as SelNode);
    }
    get startContainer() {
      return this._sc;
    }
    get startOffset() {
      return this._so;
    }
    get endContainer() {
      return this._ec;
    }
    get endOffset() {
      return this._eo;
    }
    get collapsed() {
      return this._sc === this._ec && this._so === this._eo;
    }
    get commonAncestorContainer() {
      let a: SelNode | null = this._sc;
      while (a && !holds(a, this._ec)) a = a.parentNode;
      return a ?? this._sc;
    }

    private put(start: boolean, [n, o]: Point) {
      if (start) {
        this._sc = n;
        this._so = o;
        if (rootOf(n) !== rootOf(this._ec) || compare(n, o, this._ec, this._eo) > 0) {
          this._ec = n;
          this._eo = o;
        }
      } else {
        this._ec = n;
        this._eo = o;
        if (rootOf(n) !== rootOf(this._sc) || compare(n, o, this._sc, this._so) < 0) {
          this._sc = n;
          this._so = o;
        }
      }
      this.sync();
    }
    /** @internal A range that is a selection's moves it. */
    sync() {
      const sel = owners.get(this);
      if (sel && sel._range === this) sel._push(this._sc, this._so, this._ec, this._eo);
    }
    setStart(node: unknown, offset: unknown) {
      this.put(true, point(node, offset, "setStart", "Range"));
    }
    setEnd(node: unknown, offset: unknown) {
      this.put(false, point(node, offset, "setEnd", "Range"));
    }
    setStartBefore(node: unknown) {
      this.put(true, parentPoint(node, false, "setStartBefore"));
    }
    setStartAfter(node: unknown) {
      this.put(true, parentPoint(node, true, "setStartAfter"));
    }
    setEndBefore(node: unknown) {
      this.put(false, parentPoint(node, false, "setEndBefore"));
    }
    setEndAfter(node: unknown) {
      this.put(false, parentPoint(node, true, "setEndAfter"));
    }
    collapse(toStart: unknown = false) {
      if (toStart) {
        this._ec = this._sc;
        this._eo = this._so;
      } else {
        this._sc = this._ec;
        this._so = this._eo;
      }
      this.sync();
    }
    selectNode(node: unknown) {
      const [p, i] = parentPoint(node, false, "selectNode");
      this._sc = this._ec = p;
      this._so = i;
      this._eo = i + 1;
      this.sync();
    }
    selectNodeContents(node: unknown) {
      const [n] = point(node, 0, "selectNodeContents", "Range");
      this._sc = this._ec = n;
      this._so = 0;
      this._eo = lengthOf(n);
      this.sync();
    }
    compareBoundaryPoints(how: unknown, source: RangeImpl) {
      const h = Number(how);
      if (!(h >= 0 && h <= 3)) {
        throw env.error(
          "compareBoundaryPoints",
          "Range",
          "NotSupportedError",
          "The comparison method provided must be one of 'START_TO_START', 'START_TO_END', 'END_TO_END', or 'END_TO_START'.",
        );
      }
      if (rootOf(this._sc) !== rootOf(source._sc)) {
        throw env.error(
          "compareBoundaryPoints",
          "Range",
          "WrongDocumentError",
          "The source range is in a different document than this range.",
        );
      }
      const [tn, to] = h === 0 || h === 3 ? [this._sc, this._so] : [this._ec, this._eo];
      const [sn, so] = h === 0 || h === 1 ? [source._sc, source._so] : [source._ec, source._eo];
      return compare(tn, to, sn, so);
    }
    comparePoint(node: unknown, offset: unknown) {
      const [n, o] = point(node, offset, "comparePoint", "Range");
      if (rootOf(n) !== rootOf(this._sc)) {
        throw env.error(
          "comparePoint",
          "Range",
          "WrongDocumentError",
          "The node provided and the Range are not in the same tree.",
        );
      }
      if (compare(n, o, this._sc, this._so) < 0) return -1;
      if (compare(n, o, this._ec, this._eo) > 0) return 1;
      return 0;
    }
    isPointInRange(node: unknown, offset: unknown) {
      if (!env.isNode(node) || rootOf(node) !== rootOf(this._sc)) return false;
      const [n, o] = point(node, offset, "isPointInRange", "Range");
      return compare(n, o, this._sc, this._so) >= 0 && compare(n, o, this._ec, this._eo) <= 0;
    }
    intersectsNode(node: unknown) {
      if (!env.isNode(node) || rootOf(node) !== rootOf(this._sc)) return false;
      const p = node.parentNode;
      if (!p) return true;
      const i = indexOf(node);
      return compare(p, i, this._ec, this._eo) < 0 && compare(p, i + 1, this._sc, this._so) > 0;
    }
    cloneRange() {
      const r = new RangeImpl();
      r._sc = this._sc;
      r._so = this._so;
      r._ec = this._ec;
      r._eo = this._eo;
      return r;
    }
    detach() {}
    toString() {
      const { _sc: sc, _so: so, _ec: ec, _eo: eo } = this;
      if (sc === ec && isText(sc)) return sc.kind === 2 ? (sc.text ?? "").slice(so, eo) : "";
      let out = sc.kind === 2 ? (sc.text ?? "").slice(so) : "";
      for (const n of contained(this)) out += textOf(n);
      if (ec.kind === 2) out += (ec.text ?? "").slice(0, eo);
      return out;
    }
    getClientRects() {
      const out: DOMRectLike[] = [];
      const seen = new Set<SelNode>();
      const add = (n: SelNode | null) => {
        const el = n && (n.kind === 1 ? n : n.parentNode);
        if (!el || seen.has(el)) return;
        seen.add(el);
        out.push(env.rect(el));
      };
      const partial = isText(this._sc) || isText(this._ec);
      const whole = contained(this);
      if (partial || !whole.length) {
        env.warn(
          "range:rects",
          "Range.getBoundingClientRect() / getClientRects(): a range inside a text answers its element's box — " +
            "vue-native keeps no glyph boxes outside a selection drag",
        );
      }
      if (isText(this._sc)) add(this._sc);
      for (const n of whole) add(n);
      if (isText(this._ec)) add(this._ec);
      if (!out.length) add(this._sc.kind === 1 ? (this._sc.childNodes[this._so] ?? this._sc) : this._sc);
      return out;
    }
    getBoundingClientRect() {
      const rects = this.getClientRects();
      if (!rects.length) return env.makeRect(0, 0, 0, 0);
      let [l, t, r, b] = [Infinity, Infinity, -Infinity, -Infinity];
      for (const x of rects) {
        l = Math.min(l, x.left);
        t = Math.min(t, x.top);
        r = Math.max(r, x.right);
        b = Math.max(b, x.bottom);
      }
      return env.makeRect(l, t, r - l, b - t);
    }
    deleteContents() {
      if (this.collapsed) return;
      const { _sc: sc, _so: so, _ec: ec, _eo: eo } = this;
      if (sc === ec && isText(sc)) {
        (sc as Mutable).data = (sc.text ?? "").slice(0, so) + (sc.text ?? "").slice(eo);
        this.collapse(true);
        return;
      }
      let next: Point;
      if (holds(sc, ec)) next = [sc, so];
      else {
        let ref = sc;
        while (ref.parentNode && !holds(ref.parentNode, ec)) ref = ref.parentNode;
        next = [ref.parentNode ?? sc, ref.parentNode ? indexOf(ref) + 1 : so];
      }
      const gone = contained(this);
      if (isText(sc)) (sc as Mutable).data = (sc.text ?? "").slice(0, so);
      for (const n of gone) (n.parentNode as Mutable | null)?.removeChild(n);
      if (isText(ec)) (ec as Mutable).data = (ec.text ?? "").slice(eo);
      this._sc = this._ec = next[0];
      this._so = this._eo = next[1];
      this.sync();
    }
    extractContents() {
      const frag = env.createFragment() as Mutable;
      if (this.collapsed) return frag;
      const { _sc: sc, _so: so, _ec: ec, _eo: eo } = this;
      if (sc === ec && isText(sc)) {
        frag.appendChild(env.createText((sc.text ?? "").slice(so, eo)));
        this.deleteContents();
        return frag;
      }
      adviseStructure(this, "extractContents");
      if (isText(sc)) frag.appendChild(env.createText((sc.text ?? "").slice(so)));
      const moved = contained(this);
      const tail = isText(ec) ? env.createText((ec.text ?? "").slice(0, eo)) : null;
      this.deleteContents();
      for (const n of moved) frag.appendChild(n);
      if (tail) frag.appendChild(tail);
      return frag;
    }
    cloneContents() {
      const frag = env.createFragment() as Mutable;
      if (!this.collapsed) {
        if (contained(this).some((n) => n.kind === 1)) {
          env.warn(
            "range:clone",
            "Range.cloneContents() copies the range's text only (one Text node): vue-native cannot copy elements — " +
              "build the copy from range.toString() or from the nodes themselves",
          );
        }
        frag.appendChild(env.createText(this.toString()));
      }
      return frag;
    }
    insertNode(node: unknown) {
      if (!env.isNode(node)) {
        throw new TypeError("Failed to execute 'insertNode' on 'Range': parameter 1 is not of type 'Node'.");
      }
      const { _sc: sc, _so: so } = this;
      let parent: SelNode;
      let ref: SelNode | null;
      if (isText(sc)) {
        parent = sc.parentNode!;
        if (!parent)
          throw env.error("insertNode", "Range", "HierarchyRequestError", "The range's start has no parent.");
        if (so === 0) ref = sc;
        else if (so >= (sc.text ?? "").length) ref = parent.childNodes[indexOf(sc) + 1] ?? null;
        else {
          const tail = env.createText((sc.text ?? "").slice(so));
          (sc as Mutable).data = (sc.text ?? "").slice(0, so);
          (parent as Mutable).insertBefore(tail, parent.childNodes[indexOf(sc) + 1] ?? null);
          ref = tail;
        }
      } else {
        parent = sc;
        ref = sc.childNodes[so] ?? null;
      }
      const wasCollapsed = this.collapsed;
      (parent as Mutable).insertBefore(node, ref);
      if (wasCollapsed) {
        this._ec = parent;
        this._eo = indexOf(node) + 1;
      }
      this.sync();
    }
    surroundContents(newParent: unknown) {
      if (!env.isNode(newParent)) {
        throw new TypeError("Failed to execute 'surroundContents' on 'Range': parameter 1 is not of type 'Node'.");
      }
      const frag = this.extractContents();
      while (newParent.childNodes.length) (newParent as Mutable).removeChild(newParent.childNodes[0]);
      this.insertNode(newParent);
      (newParent as Mutable).appendChild(frag);
      this.selectNode(newParent);
    }
  }
  for (const [k, v] of [
    ["START_TO_START", 0],
    ["START_TO_END", 1],
    ["END_TO_END", 2],
    ["END_TO_START", 3],
  ] as const) {
    Object.defineProperty(RangeImpl.prototype, k, { value: v });
  }
  Object.defineProperty(RangeImpl, "name", { value: "Range" });

  function contained(r: RangeImpl): SelNode[] {
    const out: SelNode[] = [];
    const common = r.commonAncestorContainer;
    const visit = (n: SelNode) => {
      for (const c of real(n).childNodes) {
        const before = compare(c.parentNode!, indexOf(c), r._sc, r._so) < 0;
        const after = compare(c.parentNode!, indexOf(c) + 1, r._ec, r._eo) > 0;
        if (!before && !after) out.push(c);
        else if (holds(c, r._sc) || holds(c, r._ec)) visit(c);
      }
    };
    if (!isText(common)) visit(common);
    return out;
  }
  function textOf(n: SelNode): string {
    if (n.kind === 2) return n.text ?? "";
    if (n.kind !== 1) return "";
    if (n.ownText && !n.childNodes.length) return n.ownText;
    let s = "";
    for (const c of n.childNodes) s += textOf(c);
    return s;
  }
  /** A partly-contained ELEMENT would be split (cloned) by a browser; here its nodes stay where they are. */
  function adviseStructure(r: RangeImpl, method: string) {
    const common = r.commonAncestorContainer;
    const split = (n: SelNode) => n !== common && !isText(n);
    if (
      split(r._sc) ||
      split(r._ec) ||
      (isText(r._sc) && r._sc.parentNode !== common) ||
      (isText(r._ec) && r._ec.parentNode !== common)
    ) {
      env.warn(
        `range:${method}`,
        `Range.${method}() across element boundaries: the elements the range cuts through are not split ` +
          "(vue-native cannot copy elements); only the wholly-contained nodes and the cut texts move",
      );
    }
  }

  function read(): [SelNode, number, SelNode, number] | null {
    let raw = host!.get();
    if (!raw) return null;
    let p = raw.split(",").map(Number);
    if (p[2] || p[5]) {
      // A boundary in an element's own text: give it its Text child (the host
      // moves the boundary with it), then read again.
      for (const at of [0, 3]) {
        const el = p[at + 2] ? env.nodeById(p[at]!) : undefined;
        if (el) env.materialize(el);
      }
      raw = host!.get();
      if (!raw) return null;
      p = raw.split(",").map(Number);
    }
    const a = env.nodeById(p[0]!);
    const f = env.nodeById(p[3]!);
    if (!a || !f) return null;
    return [a, p[1]!, f, p[4]!];
  }

  class SelectionImpl {
    /** @internal The range `getRangeAt` / `addRange` tied to this selection. */
    _range: RangeImpl | null = null;

    /** @internal */
    _push(an: SelNode, ao: number, fn: SelNode, fo: number) {
      host!.set(an.__vnId, ao, fn.__vnId, fo);
    }
    get anchorNode() {
      return read()?.[0] ?? null;
    }
    get anchorOffset() {
      return read()?.[1] ?? 0;
    }
    get focusNode() {
      return read()?.[2] ?? null;
    }
    get focusOffset() {
      return read()?.[3] ?? 0;
    }
    get baseNode() {
      return this.anchorNode;
    }
    get baseOffset() {
      return this.anchorOffset;
    }
    get extentNode() {
      return this.focusNode;
    }
    get extentOffset() {
      return this.focusOffset;
    }
    get isCollapsed() {
      const s = read();
      return !s || (s[0] === s[2] && s[1] === s[3]);
    }
    get rangeCount() {
      return read() ? 1 : 0;
    }
    get type() {
      const s = read();
      return !s ? "None" : s[0] === s[2] && s[1] === s[3] ? "Caret" : "Range";
    }
    get direction() {
      const s = read();
      if (!s || (s[0] === s[2] && s[1] === s[3])) return "none";
      return compare(s[0], s[1], s[2], s[3]) <= 0 ? "forward" : "backward";
    }
    getRangeAt(index: unknown) {
      const s = read();
      if (!s || Number(index) !== 0) {
        throw env.error("getRangeAt", "Selection", "IndexSizeError", `${Number(index)} is not a valid index.`);
      }
      const back = compare(s[0], s[1], s[2], s[3]) > 0;
      const [sn, so, en, eo] = back ? [s[2], s[3], s[0], s[1]] : s;
      const r = this._range;
      if (r && r._sc === sn && r._so === so && r._ec === en && r._eo === eo) return r;
      const fresh = new RangeImpl();
      fresh._sc = sn;
      fresh._so = so;
      fresh._ec = en;
      fresh._eo = eo;
      this.tie(fresh);
      return fresh;
    }
    private tie(r: RangeImpl | null) {
      this._range = r;
      if (r) owners.set(r, this);
    }
    addRange(range: unknown) {
      if (!(range instanceof RangeImpl)) {
        throw new TypeError("Failed to execute 'addRange' on 'Selection': parameter 1 is not of type 'Range'.");
      }
      if (read() || !inDocument(range._sc)) return;
      this.tie(range);
      this._push(range._sc, range._so, range._ec, range._eo);
    }
    removeRange(range: unknown) {
      if (range === this._range || (range instanceof RangeImpl && this.rangeCount && this.same(range))) {
        this.removeAllRanges();
        return;
      }
      throw env.error("removeRange", "Selection", "NotFoundError", "The given range was not found.");
    }
    private same(r: RangeImpl) {
      const cur = this.getRangeAt(0);
      return cur._sc === r._sc && cur._so === r._so && cur._ec === r._ec && cur._eo === r._eo;
    }
    removeAllRanges() {
      this.tie(null);
      host!.set(0, 0, 0, 0);
    }
    empty() {
      this.removeAllRanges();
    }
    collapse(node: unknown, offset: unknown = 0) {
      if (node == null) {
        this.removeAllRanges();
        return;
      }
      const [n, o] = point(node, offset, "collapse", "Selection");
      if (!inDocument(n)) return;
      this.tie(null);
      this._push(n, o, n, o);
    }
    setPosition(node: unknown, offset: unknown = 0) {
      this.collapse(node, offset);
    }
    collapseToStart() {
      const r = this.rangeOrThrow("collapseToStart");
      this.collapse(r._sc, r._so);
    }
    collapseToEnd() {
      const r = this.rangeOrThrow("collapseToEnd");
      this.collapse(r._ec, r._eo);
    }
    private rangeOrThrow(method: string) {
      if (!this.rangeCount) throw env.error(method, "Selection", "InvalidStateError", "there is no selection.");
      return this.getRangeAt(0);
    }
    extend(node: unknown, offset: unknown = 0) {
      const s = read();
      if (!s)
        throw env.error("extend", "Selection", "InvalidStateError", "This Selection object doesn't have any Ranges.");
      const [n, o] = point(node, offset, "extend", "Selection");
      if (!inDocument(n)) return;
      this.tie(null);
      this._push(s[0], s[1], n, o);
    }
    setBaseAndExtent(anchorNode: unknown, anchorOffset: unknown, focusNode: unknown, focusOffset: unknown) {
      const [a, ao] = point(anchorNode, anchorOffset, "setBaseAndExtent", "Selection");
      const [f, fo] = point(focusNode, focusOffset, "setBaseAndExtent", "Selection");
      if (!inDocument(a) || !inDocument(f)) return;
      this.tie(null);
      this._push(a, ao, f, fo);
    }
    selectAllChildren(node: unknown) {
      const [n] = point(node, 0, "selectAllChildren", "Selection");
      if (!inDocument(n)) return;
      this.tie(null);
      this._push(n, 0, n, lengthOf(n));
    }
    containsNode(node: unknown, allowPartial: unknown = false) {
      if (!env.isNode(node) || !this.rangeCount) return false;
      const r = this.getRangeAt(0);
      if (rootOf(node) !== rootOf(r._sc)) return false;
      const len = lengthOf(node);
      if (!allowPartial) return compare(r._sc, r._so, node, 0) <= 0 && compare(r._ec, r._eo, node, len) >= 0;
      return compare(r._sc, r._so, node, len) <= 0 && compare(r._ec, r._eo, node, 0) >= 0;
    }
    deleteFromDocument() {
      if (!this.rangeCount) return;
      this.getRangeAt(0).deleteContents();
    }
    modify() {
      env.warn(
        "selection:modify",
        "Selection.modify() is not implemented — vue-native has no caret movement outside text fields; " +
          "set the boundaries with collapse() / extend() instead",
      );
    }
    toString() {
      return host!.text();
    }
  }
  Object.defineProperty(SelectionImpl, "name", { value: "Selection" });

  const selection = new SelectionImpl();
  const getSelection = () => selection;
  // `new Selection()` is an illegal constructor in a browser; `instanceof` still works.
  const SelectionIface = function Selection() {
    throw new TypeError("Illegal constructor");
  } as unknown as { prototype: object };
  Object.defineProperty(SelectionIface, "prototype", { value: SelectionImpl.prototype });
  Object.defineProperty(SelectionImpl.prototype, "constructor", {
    value: SelectionIface,
    configurable: true,
    writable: true,
  });

  env.defineGlobal("Range", RangeImpl);
  env.defineGlobal("Selection", SelectionIface);
  env.defineGlobal("getSelection", getSelection);
  env.defineDoc("getSelection", getSelection);
  env.defineDoc("createRange", () => new RangeImpl());
}
