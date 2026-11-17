// Dev-build performance advisories. Every caller guards with `if (__DEV__)`, so a production bundle has none of
// this. Vue's dev build reports each component's render and patch to its devtools hook, which is where an update
// is measured; the host writes it caused are counted at the write sites.

import type { MirrorNode } from "./types";
import { CODE_TO_NAME } from "./events";

const WASTED_MIN_ELEMENTS = 500;
const WASTED_MAX_CHANGED = 0.05;
const LIST_MIN_ITEMS = 50;
const LIST_MAX_IN_VIEW = 0.25;
const WRITES_PER_FRAME = 1000;

interface DevtoolsHookLike {
  enabled: boolean;
  appRecords: never[];
  on(): void;
  once(): void;
  off(): void;
  emit(event: string, ...payload: unknown[]): void;
}

/** What the render watcher uses of Vue, handed in by the Vue renderer: the DOM surface imports this module too. */
export interface VueInternals {
  Fragment: unknown;
  isRef(v: unknown): v is { value: unknown };
  toRaw<T>(v: T): T;
  setDevtoolsHook(hook: DevtoolsHookLike, target: object): void;
}

interface DebuggerEvent {
  target?: unknown;
  key?: unknown;
}

interface VNode {
  type: unknown;
  patchFlag: number;
  shapeFlag: number;
  children: unknown;
  el: unknown;
  suspense?: { activeBranch: VNode | null } | null;
}

interface Instance {
  type: unknown;
  isMounted: boolean;
  subTree: VNode;
  props: unknown;
  data: unknown;
  effect: { onTrigger?: ((e: DebuggerEvent) => void) | undefined };
}

let vue: VueInternals | null = null;
const isRef = (v: unknown): v is { value: unknown } => vue?.isRef(v) ?? false;
const toRaw = (v: unknown): unknown => (vue ? vue.toRaw(v) : v);

interface Trigger {
  target: unknown;
  key: unknown;
  cause: string;
  computeds: string[];
}

interface Open {
  instance: Instance;
  start: number;
  changed: number;
  writes: number;
  childChanged: number;
  childWrites: number;
  childMs: number;
}

interface ListFound {
  items: number;
  elements: number;
  parent: number;
}

interface Finding {
  instance: Instance;
  elements: number;
  changed: number;
  ms: number;
  triggers: Trigger[];
  list: ListFound | null;
}

interface State {
  stamps: WeakMap<object, number>;
  serial: number;
  changed: number;
  writes: number;
  open: Open[];
  pending: WeakMap<Instance, Trigger[]>;
  batch: { writes: number; byComponent: Map<Instance, number>; findings: Finding[] } | null;
  frame: { id: number; writes: number; byComponent: Map<Instance, number>; reported: boolean };
  vue: boolean;
}

let state: State | null = null;
let cause = "";

function start(): State {
  state ??= {
    stamps: new WeakMap(),
    serial: 0,
    changed: 0,
    writes: 0,
    open: [],
    pending: new WeakMap(),
    batch: null,
    frame: { id: -1, writes: 0, byComponent: new Map(), reported: false },
    vue: false,
  };
  return state;
}

const hostReady = (): boolean =>
  typeof __sk_perf_advise === "function" &&
  typeof __sk_perf_frame === "function" &&
  typeof __sk_perf_visible === "function";

/** Counts the writes the DOM surface makes (Svelte, direct DOM code), for the host-writes advisory. */
export function watchDomWrites(): void {
  if (hostReady()) start();
}

/** Measures every Vue component update. Call after the renderer exists: creating one resets Vue's hook. */
export function watchVueRenders(internals: VueInternals): void {
  if (!hostReady()) return;
  vue = internals;
  start().vue = true;
  internals.setDevtoolsHook(
    { enabled: true, appRecords: [], on() {}, once() {}, off() {}, emit: onVueEvent },
    globalThis,
  );
}

/** The host event being dispatched while `dispatch` runs, so a state change can name what caused it. */
export function withCause(dispatch: SkDispatcher): SkDispatcher {
  return (node, code, a, b, c, d, key, data, x, y) => {
    const outer = cause;
    cause = CODE_TO_NAME[code] ?? "";
    try {
      return dispatch(node, code, a, b, c, d, key, data, x, y);
    } finally {
      cause = outer;
    }
  };
}

function batch(s: State): NonNullable<State["batch"]> {
  if (!s.batch) {
    s.batch = { writes: 0, byComponent: new Map(), findings: [] };
    s.serial++;
    void Promise.resolve().then(closeBatch);
  }
  return s.batch;
}

function stamp(s: State, node: object): void {
  if (s.stamps.get(node) === s.serial) return;
  s.stamps.set(node, s.serial);
  s.changed++;
}

/** A style, class, attribute or text write on `node` reached the host. Writes before the node is attached are part of creating it and are not counted. */
export function noteWrite(node: MirrorNode): void {
  const s = state;
  if (!s) return;
  const parent = node.parentNode;
  if (!parent) return;
  const b = batch(s);
  s.writes++;
  b.writes++;
  stamp(s, node.kind === 1 ? node : parent);
}

/** A child of `parent` was inserted or removed. */
export function noteChildren(parent: MirrorNode | null): void {
  const s = state;
  if (!s || !parent) return;
  batch(s);
  stamp(s, parent);
}

function onVueEvent(event: string, ...payload: unknown[]): void {
  const s = state;
  if (!s) return;
  if (event === "perf:start" || event === "perf:end") {
    const instance = payload[2] as Instance;
    const type = payload[3];
    const time = typeof payload[4] === "number" ? payload[4] : 0;
    if (!instance.isMounted) return;
    if (event === "perf:start" && type === "render") openUpdate(s, instance, time);
    else if (event === "perf:end" && type === "patch") closeUpdate(s, instance, time);
  } else if (event === "component:added") {
    watchTriggers(s, payload[3] as Instance);
  }
}

function watchTriggers(s: State, instance: Instance): void {
  const effect = instance.effect;
  const outer = effect.onTrigger;
  effect.onTrigger = (e: DebuggerEvent) => {
    outer?.(e);
    let list = s.pending.get(instance);
    if (!list) s.pending.set(instance, (list = []));
    if (list.length >= 8) return;
    // A computed passes its change on without saying what it was; the computeds the template holds that are dirty
    // right now are the ones that changed, and by the time anything else could look they are clean again.
    list.push({ target: e.target, key: e.key, cause, computeds: e.target ? [] : dirtyComputeds(instance) });
  };
}

function openUpdate(s: State, instance: Instance, time: number): void {
  batch(s);
  s.open.push({
    instance,
    start: time,
    changed: s.changed,
    writes: s.writes,
    childChanged: 0,
    childWrites: 0,
    childMs: 0,
  });
}

function closeUpdate(s: State, instance: Instance, time: number): void {
  let open: Open | undefined;
  while ((open = s.open.pop()) && open.instance !== instance);
  if (!open) return;
  const changed = s.changed - open.changed;
  const writes = s.writes - open.writes;
  const ms = time - open.start;
  const outer = s.open[s.open.length - 1];
  if (outer) {
    outer.childChanged += changed;
    outer.childWrites += writes;
    outer.childMs += ms;
  }
  const b = batch(s);
  const ownWrites = writes - open.childWrites;
  if (ownWrites > 0) b.byComponent.set(instance, (b.byComponent.get(instance) ?? 0) + ownWrites);
  const triggers = s.pending.get(instance) ?? [];
  s.pending.delete(instance);
  const walk: Walk = { list: null };
  const elements = countElements(instance.subTree, walk);
  if (elements < WASTED_MIN_ELEMENTS) return;
  b.findings.push({
    instance,
    elements,
    changed: changed - open.childChanged,
    ms: ms - open.childMs,
    triggers,
    list: walk.list,
  });
}

interface Walk {
  list: ListFound | null;
}

const ELEMENT = 1;
const COMPONENT = 6;
const SUSPENSE = 128;
const LIST_FRAGMENT = 128 | 256;
const CACHED = -1;

/** The elements a component's render made, its child components' excluded, and its largest list. */
function countElements(vnode: VNode, walk: Walk): number {
  if (vnode.patchFlag === CACHED || vnode.shapeFlag & COMPONENT) return 0;
  if (vnode.shapeFlag & SUSPENSE) {
    const branch = vnode.suspense?.activeBranch;
    return branch ? countElements(branch, walk) : 0;
  }
  const isElement = (vnode.shapeFlag & ELEMENT) !== 0;
  let count = isElement ? 1 : 0;
  const kids = vnode.children;
  if (!Array.isArray(kids)) return count;
  let inside = 0;
  for (const kid of kids) if (isVNode(kid)) inside += countElements(kid, walk);
  count += inside;
  const isList = isElement || (vnode.type === vue?.Fragment && (vnode.patchFlag & LIST_FRAGMENT) !== 0);
  if (isList && kids.length >= LIST_MIN_ITEMS && (!walk.list || inside > walk.list.elements)) {
    const el = vnode.el as MirrorNode | null;
    const parent = isElement ? el : (el?.parentNode ?? null);
    if (parent) walk.list = { items: kids.length, elements: inside, parent: parent.__skId };
  }
  return count;
}

const isVNode = (v: unknown): v is VNode =>
  typeof v === "object" && v !== null && (v as { __v_isVNode?: unknown }).__v_isVNode === true;

function closeBatch(): void {
  const s = state;
  const b = s?.batch;
  if (!s || !b) return;
  s.batch = null;
  s.open.length = 0;
  for (const f of b.findings) {
    adviseWasted(f);
    if (f.list) adviseList(f, f.list);
  }
  const id = __sk_perf_frame();
  if (s.frame.id !== id) s.frame = { id, writes: 0, byComponent: new Map(), reported: false };
  const frame = s.frame;
  frame.writes += b.writes;
  for (const [instance, n] of b.byComponent)
    frame.byComponent.set(instance, (frame.byComponent.get(instance) ?? 0) + n);
  if (!frame.reported && frame.writes >= WRITES_PER_FRAME) {
    frame.reported = true;
    adviseWrites(s, frame.writes, frame.byComponent);
  }
}

function adviseWasted(f: Finding): void {
  if (f.changed >= f.elements * WASTED_MAX_CHANGED) return;
  const causes = new Set(f.triggers.map((t) => t.cause).filter(Boolean));
  const on = causes.size ? ` on ${[...causes].join(", ")}` : "";
  const deps = [...new Set(f.triggers.flatMap((t) => dependencyNames(f.instance, t)))].slice(0, 4);
  const why = deps.length ? ` It was triggered by ${deps.join(", ")}.` : "";
  const scrolled = causes.has("scroll") || causes.has("wheel") || deps.some((d) => /scroll/i.test(d));
  const fix = scrolled
    ? "A value that follows the scroll position makes the whole template re-render on every scroll event: keep " +
      "headers in view with position: sticky, or move them with a transform set through a template ref in the " +
      "scroll handler, so the template does not read the scroll offset."
    : "Every element re-renders when state that only a few of them show changes (a selection, a hover, the item " +
      "being edited): draw that state as one overlay element positioned over the list, or move the repeated " +
      "element into a child component that receives only what it shows as props, so only the ones whose props " +
      "change update." +
      (f.list
        ? ` Rendering only the items in view (virtualizing the list of ${f.list.items}) also bounds the cost.`
        : "");
  __sk_perf_advise(
    "wasted-render",
    `${componentKey(f.instance)}|${scrolled ? "scroll" : [...deps].sort().join()}`,
    `${componentLabel(f.instance)} re-rendered ${f.elements} elements${on} and ${f.changed} of them changed ` +
      `(${f.ms.toFixed(1)} ms).${why} ${fix}`,
  );
}

function adviseList(f: Finding, list: ListFound): void {
  const answer = __sk_perf_visible(list.parent);
  const comma = answer.indexOf(",");
  if (comma < 0) return;
  const total = Number(answer.slice(0, comma));
  const seen = Number(answer.slice(comma + 1));
  if (total < WASTED_MIN_ELEMENTS || seen >= total * LIST_MAX_IN_VIEW) return;
  __sk_perf_advise(
    "long-list",
    `${componentKey(f.instance)}|${list.items}`,
    `${componentLabel(f.instance)} re-rendered a list of ${list.items} items (${total} elements) inside a scroll ` +
      `container that shows ${seen} of those elements, and each update diffs all of them. Render only the items ` +
      `in view (virtualize): a spacer as tall as the whole list holding a v-for over the visible slice, ` +
      `recomputed from scrollTop.`,
  );
}

function adviseWrites(s: State, writes: number, byComponent: Map<Instance, number>): void {
  const top = [...byComponent].sort((a, b) => b[1] - a[1]);
  let attributed = 0;
  for (const [, n] of top) attributed += n;
  const parts = top.slice(0, 3).map(([instance, n]) => `${n} from ${componentLabel(instance)}`);
  if (s.vue && writes > attributed) parts.push(`${writes - attributed} from code outside a Vue component update`);
  const from = parts.length ? `: ${parts.join(", ")}` : "";
  const first = top[0];
  __sk_perf_advise(
    "host-writes",
    first ? componentKey(first[0]) : "dom",
    `${writes} style, class, attribute and text writes reached the native tree in one frame${from}. When many ` +
      `elements take the same new value, set it once on a shared parent (a class, or a CSS variable the children ` +
      `read); animate with transform or element.animate() instead of writing styles every frame.`,
  );
}

interface ComponentMeta {
  name?: unknown;
  __name?: unknown;
  __file?: unknown;
}

function componentKey(instance: Instance): string {
  const type = instance.type as ComponentMeta;
  return typeof type.__file === "string" ? type.__file : componentName(instance);
}

function componentName(instance: Instance): string {
  const type = instance.type as ComponentMeta;
  if (typeof type.name === "string" && type.name) return type.name;
  if (typeof type.__name === "string" && type.__name) return type.__name;
  const file = typeof type.__file === "string" ? type.__file : "";
  return /([^/\\]+)\.\w+$/.exec(file)?.[1] ?? "Anonymous";
}

function componentLabel(instance: Instance): string {
  const type = instance.type as ComponentMeta;
  const file = typeof type.__file === "string" ? ` (${type.__file})` : "";
  return `<${componentName(instance)}>${file}`;
}

function setupBindings(instance: Instance): Record<string, unknown> | null {
  const raw = (instance as { devtoolsRawSetupState?: unknown }).devtoolsRawSetupState;
  return typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>) : null;
}

const isComputed = (v: unknown): boolean => isRef(v) && typeof (v as { fn?: unknown }).fn === "function";
const DIRTY = 16;

function dirtyComputeds(instance: Instance): string[] {
  const bindings = setupBindings(instance);
  if (!bindings) return [];
  const out: string[] = [];
  for (const name in bindings) {
    const v = bindings[name];
    if (isComputed(v) && ((v as { flags?: number }).flags ?? 0) & DIRTY) out.push(name);
  }
  return out;
}

function member(name: string, key: unknown): string {
  if (typeof key === "symbol" || key === undefined) return name;
  const k = String(key);
  return /^\d+$/.test(k) ? `${name}[${k}]` : `${name}.${k}`;
}

/** What a trigger changed, under the name the component's template knows it by where there is one. */
function dependencyNames(instance: Instance, t: Trigger): string[] {
  if (t.target === undefined) return t.computeds.length ? t.computeds : ["a computed value"];
  const bindings = setupBindings(instance);
  if (bindings) {
    for (const name in bindings) {
      const v = bindings[name];
      if (v === t.target) return [name];
      if (typeof v !== "object" || v === null) continue;
      if (toRaw(v) === t.target) return [member(name, t.key)];
      if (isRef(v) && !isComputed(v)) {
        const inner: unknown = v.value;
        if (typeof inner === "object" && inner !== null && toRaw(inner) === t.target) return [member(name, t.key)];
      }
    }
  }
  if (instance.props && toRaw(instance.props) === t.target) return [member("props", t.key)];
  if (instance.data && toRaw(instance.data) === t.target) return [member("this", t.key)];
  if (isRef(t.target)) return ["a ref read inside a function the template calls"];
  return [typeof t.key === "symbol" ? "a reactive collection" : `a reactive object's "${String(t.key)}"`];
}
