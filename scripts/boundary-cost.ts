// What does crossing the `__sk_*` boundary cost, on this machine?
//
//   bun scripts/boundary-cost.ts [app]        default: hello
//
// Runs inside the real host (`sk-test`), so the numbers include FFI, string marshalling and Rust
// work. Nothing here asserts; it is an instrument.

import { launch } from "../packages/test/src/index";

const APP = process.argv[2] ?? "hello";

/** One measured loop, run inside the app. Returns nanoseconds per operation. */
const bench = (label: string, n: number, body: string, setup = "") => ({
  label,
  n,
  js: `(() => {
    ${setup}
    for (let i = 0; i < Math.min(${n}, 2000); i++) { ${body} }
    const t0 = performance.now();
    for (let i = 0; i < ${n}; i++) { ${body} }
    const t1 = performance.now();
    return (t1 - t0) * 1e6 / ${n};
  })()`,
});

const BENCHES = [
  // -- the floor: a host fn taking one integer and returning the empty string.
  bench("__sk_bounds(0) — the crossing floor", 200_000, `__sk_bounds(0);`),
  bench("__sk_intern (hot: name already interned)", 200_000, `__sk_intern("div");`),
  bench(
    "__sk_set_style (detached node)",
    100_000,
    `__sk_set_style(scratch, key, "12px");`,
    `const scratch = __sk_create_element(__sk_intern("div")); const key = __sk_intern("padding-left");`,
  ),
  bench(
    "__sk_bounds + parse (what getBoundingClientRect costs)",
    50_000,
    `el.getBoundingClientRect();`,
    `const el = document.documentElement;`,
  ),
  bench("__sk_bounds (raw, no parse)", 50_000, `__sk_bounds(1);`),
  bench(
    "getComputedStyle(el).display (one property)",
    20_000,
    `void getComputedStyle(el).display;`,
    `const el = document.documentElement;`,
  ),
  bench(
    "el.style.paddingLeft = … (a named property write)",
    100_000,
    `el.style.paddingLeft = i % 2 ? "12px" : "13px";`,
    `const el = document.createElement("div");`,
  ),
  bench(
    "dispatchEvent: synthesis + walk (what an event really costs)",
    100_000,
    `el.dispatchEvent(new MouseEvent("click", { bubbles: true }));`,
    `const el = deepest;`,
  ),
  bench(
    "…of which: new MouseEvent(…) alone",
    100_000,
    `void new MouseEvent("click", { bubbles: true, clientX: 1, clientY: 2, button: 0 });`,
  ),
  bench(
    "…of which: the propagation walk alone (one reused event)",
    100_000,
    `el.dispatchEvent(ev);`,
    `const el = deepest; const ev = new MouseEvent("click", { bubbles: true });`,
  ),
  bench(
    "Object.defineProperty on an event (propagate does 2+ per dispatch)",
    200_000,
    `Object.defineProperty(ev, "currentTarget", { value: null, configurable: true });`,
    `const ev = new MouseEvent("click");`,
  ),
  bench("node.parentNode (mirror read)", 1_000_000, `void el.parentNode;`, `const el = deepest;`),
  bench(
    "node.nextSibling (mirror read, indexOf)",
    200_000,
    `void nodeOpsNextSibling(el);`,
    `const el = deepest;
     const nodeOpsNextSibling = (n) => {
       const p = n.parentNode; if (!p) return null;
       const i = p.childNodes.indexOf(n); return i >= 0 ? (p.childNodes[i + 1] ?? null) : null;
     };`,
  ),
];

const DEEPEST = `(() => {
  let best = document.documentElement, bestDepth = 0;
  const walk = (n, d) => {
    if (d > bestDepth) { best = n; bestDepth = d; }
    for (const c of n.childNodes || []) if (c.kind === 1) walk(c, d + 1);
  };
  walk(document.documentElement, 0);
  globalThis.deepest = best;
  return bestDepth;
})()`;

async function main() {
  const app = await launch({ app: APP, env: { SK_PERF: "1" }, verbose: true });
  try {
    const depth = await app.eval(DEEPEST);
    const nodes = await app.eval(
      `(() => { let n = 0; const walk = (x) => { n++; for (const c of x.childNodes || []) walk(c); };
        walk(document.documentElement); return n; })()`,
    );
    console.log(`\napp: ${APP} — ${nodes} mirror nodes, deepest element at depth ${depth}\n`);

    const width = Math.max(...BENCHES.map((b) => b.label.length));
    console.log(`${"operation".padEnd(width)} ${"n".padStart(9)} ${"ns/op".padStart(10)}`);
    console.log("-".repeat(width + 22));
    for (const b of BENCHES) {
      const ns = (await app.eval(b.js)) as number;
      console.log(`${b.label.padEnd(width)} ${String(b.n).padStart(9)} ${ns.toFixed(1).padStart(10)}`);
    }
    console.log();
  } finally {
    await app.close();
  }
}

await main();
