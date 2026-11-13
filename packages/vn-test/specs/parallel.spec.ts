// `bun test` runs SPEC FILES serially in one process, so hello.spec.ts and shadcn.spec.ts each get
// the machine to themselves and never test the thing their separate slots exist for.

import { afterAll, describe, test } from "bun:test";
import { expect, launch } from "../src/index";

const T = 60_000;

// Concurrently, on purpose: two builders, two slot copies and two host boots overlapping is the
// case that would expose a shared temp path.
const [a, b] = await Promise.all([
  launch({ app: "hello", window: [900, 700], label: "par-a" }),
  launch({ app: "hello", window: [900, 700], label: "par-b" }),
]);
afterAll(() => Promise.all([a.close(), b.close()]));

describe("parallel", () => {
  test("two hosts, two slots, two ports", () => {
    expect(a.pid).not.toBe(b.pid);
    expect(a.slotDir).not.toBe(b.slotDir);
    // The ports are what a collision would actually look like.
    expect(a.port).toBeGreaterThan(0);
    expect(b.port).toBeGreaterThan(0);
    expect(a.port).not.toBe(b.port);
  }, T);

  test("both answer, and each answers for itself", async () => {
    const [pa, pb] = await Promise.all([a.send("ping"), b.send("ping")]);
    expect(pa.pid).toBe(a.pid);
    expect(pb.pid).toBe(b.pid);
    expect(pa.pid).not.toBe(pb.pid);
  }, T);

  test("state driven into one app does not appear in the other", async () => {
    const bumpA = a.locator(".store-row .btn").first();
    const bumpB = b.locator(".store-row .btn").first();
    await expect(bumpA).toHaveText("visits++ (0)");
    await expect(bumpB).toHaveText("visits++ (0)");

    await bumpA.click();
    await bumpA.click();
    await expect(bumpA).toHaveText("visits++ (2)");
    await expect(bumpB).toHaveText("visits++ (0)");

    await bumpB.click();
    await expect(bumpB).toHaveText("visits++ (1)");
    await expect(bumpA).toHaveText("visits++ (2)");
  }, T);

  test("typing into one editor does not leak into the other", async () => {
    await a.locator(".field").first().type("alpha");
    await b.locator(".field").first().type("beta");

    await expect(a.locator(".field").first()).toHaveText("alpha");
    await expect(b.locator(".field").first()).toHaveText("beta");
    await expect(a.locator(".hint").first()).toHaveText('v-model echo: "alpha" (5 chars)');
    await expect(b.locator(".hint").first()).toHaveText('v-model echo: "beta" (4 chars)');
  }, T);

  test("commands issued at the same moment do not cross wires", async () => {
    // Interleaving ten round trips is what would surface a reply matched to the wrong request.
    const work = [];
    for (let i = 0; i < 5; i++) {
      work.push(a.locator(".bar .store").textContent());
      work.push(b.locator(".bar .store").textContent());
    }
    const results = await Promise.all(work);
    for (let i = 0; i < results.length; i += 2) {
      expect(results[i]).toBe("pinia · session · guest · 2");
      expect(results[i + 1]).toBe("pinia · session · guest · 1");
    }
  }, T);
});
