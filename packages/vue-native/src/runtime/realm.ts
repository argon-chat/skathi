// Cached before user code can replace the globals. A zero token is the inactive path.
const traceCallbackGlobals = globalThis as unknown as {
  __vn_trace_callback_begin?: (callback: unknown) => number;
  __vn_trace_callback_end?: (token: number) => void;
};
export const traceCallbackBegin =
  typeof traceCallbackGlobals.__vn_trace_callback_begin === "function" &&
  typeof traceCallbackGlobals.__vn_trace_callback_end === "function"
    ? traceCallbackGlobals.__vn_trace_callback_begin
    : undefined;
export const traceCallbackEnd = traceCallbackBegin ? traceCallbackGlobals.__vn_trace_callback_end : undefined;

// `location.reload()` and the dev host's cold restart re-evaluate this module in the SAME realm.
// REALM.installed: what a runtime installed on engine-owned objects. `vacant()` counts it as a hole, so the
//   next runtime installs its OWN class over it (a kept class runs against the old runtime's state).
interface RealmRecord {
  base: Set<PropertyKey>;
  installed: WeakSet<object>;
  runs: number;
  originals?: WeakMap<object, Map<PropertyKey, unknown>>;
}
const REALM: RealmRecord = ((globalThis as { __VN_REALM__?: RealmRecord }).__VN_REALM__ ??= {
  base: new Set(Reflect.ownKeys(globalThis)),
  installed: new WeakSet(),
  runs: 0,
});
REALM.runs++;

export function mark<T>(v: T): T {
  if ((typeof v === "object" && v !== null) || typeof v === "function") REALM.installed.add(v as object);
  return v;
}

/** `obj[name]` as the FIRST runtime in this realm found it: after a reload, wrap the original, not the old wrapper. */
export function original<T>(obj: object, name: PropertyKey, now: T): T {
  const table = (REALM.originals ??= new WeakMap());
  let byName = table.get(obj);
  if (!byName) table.set(obj, (byName = new Map()));
  if (!byName.has(name)) byName.set(name, now);
  return byName.get(name) as T;
}

/** Is `obj[name]` missing or a previous runtime's install? Walks the prototype chain by descriptor, so no getter runs. */
export function vacant(obj: object, name: PropertyKey): boolean {
  for (let o: object | null = obj; o; o = Object.getPrototypeOf(o)) {
    const d = Object.getOwnPropertyDescriptor(o, name);
    if (!d) continue;
    const v = d.get ?? d.value;
    if (v === undefined) return true;
    return (typeof v === "object" && v !== null) || typeof v === "function" ? REALM.installed.has(v) : false;
  }
  return true;
}

/** Deletes configurable globals added since the first runtime, except `__vn*`. Returns how many could not be deleted. */
export function resetGlobals(): number {
  let kept = 0;
  for (const k of Reflect.ownKeys(globalThis)) {
    if (REALM.base.has(k) || (typeof k === "string" && /^__vn/i.test(k))) continue;
    const d = Object.getOwnPropertyDescriptor(globalThis, k);
    if (!d) continue;
    if (!d.configurable) {
      kept++;
      continue;
    }
    try {
      delete (globalThis as Record<PropertyKey, unknown>)[k];
    } catch {
      kept++;
    }
  }
  return kept;
}
