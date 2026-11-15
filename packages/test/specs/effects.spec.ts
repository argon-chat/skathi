import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/shading/src/main.ts",
  window: [360, 200],
  label: "shading",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Shading = {
  id: number;
  value: string | null;
  source: string;
  effect: string | null;
  params: number[] | null;
  overridden: boolean;
  authored: string | null;
  inline: string | null;
};

const idOf = async (selector: string): Promise<number> => (await app.query(selector)).nodes[0]!.id;
const node = (id: number): Promise<Shading> => app.send("effects", { op: "node", node: id });

async function until(id: number, ok: (n: Shading) => boolean): Promise<Shading> {
  const deadline = Date.now() + 5_000;
  let n = await node(id);
  while (!ok(n) && Date.now() < deadline) {
    await sleep(50);
    n = await node(id);
  }
  return n;
}

test(
  "list carries the schemas and every element painting an effect, wherever it was declared",
  async () => {
    const list = await app.send("effects", { op: "list" });
    const builtin = list.manifests.find((m: { builtin: boolean }) => m.builtin);
    expect(builtin.manifest.effects.map((e: { name: string }) => e.name)).toContain("frost");
    expect(list.loaded.map((e: { name: string }) => e.name)).toContain("glow");

    const ids = [await idOf("#glass"), await idOf("#locked"), await idOf("#inline")];
    const byId = new Map<number, Shading>(list.nodes.map((n: Shading) => [n.id, n]));
    expect(ids.map((id) => [byId.get(id)?.source, byId.get(id)?.effect])).toEqual([
      ["rule", "frost"],
      ["rule", "noise"],
      ["inline", "glow"],
    ]);
    expect(byId.has(await idOf("#plain"))).toBe(false);
  },
  T,
);

test(
  "set overrides live and reset goes back to the rule",
  async () => {
    const id = await idOf("#glass");
    const rule = (await node(id)).value;
    const out = await app.send("effects", { op: "set", node: id, value: "frost(0.9, 20px)" });
    expect(out.applies).toBe(true);
    await app.send("effects", { op: "set", node: id, value: "frost(0.8, 20px)" });
    const edited = await until(id, (n) => n.params?.[0] === Math.fround(0.8));
    expect([edited.source, edited.value, edited.authored]).toEqual(["override", "frost(0.8, 20px)", rule]);
    expect(edited.params?.[1]).toBe(20);

    await app.send("effects", { op: "reset", node: id });
    const back = await until(id, (n) => n.params?.[0] === 0.5);
    expect([back.source, back.value, back.overridden, back.inline]).toEqual(["rule", rule, false, null]);
  },
  T,
);

test(
  "an override beats an !important rule, and reset restores the app's own inline value",
  async () => {
    const locked = await idOf("#locked");
    await app.send("effects", { op: "set", node: locked, value: "glow(12px, 1)" });
    expect((await until(locked, (n) => n.effect === "glow")).effect).toBe("glow");
    await app.send("effects", { op: "reset", node: locked });
    expect((await until(locked, (n) => n.effect === "noise")).source).toBe("rule");

    const inline = await idOf("#inline");
    await app.send("effects", { op: "set", node: inline, value: "none" });
    expect((await until(inline, (n) => n.effect === null)).source).toBe("override");
    await app.send("effects", { op: "reset", node: inline });
    const back = await until(inline, (n) => n.effect === "glow");
    expect([back.source, back.inline]).toEqual(["inline", "glow(8px, 0.5)"]);
  },
  T,
);

test(
  "an out-of-range value is applied clamped and says so",
  async () => {
    const id = await idOf("#glass");
    const out = await app.send("effects", { op: "set", node: id, value: "frost(3)" });
    expect(out.diagnostics.length).toBeGreaterThan(0);
    expect((await until(id, (n) => n.params?.[0] === 1)).params?.[0]).toBe(1);
    await app.send("effects", { op: "reset", node: id });
  },
  T,
);
