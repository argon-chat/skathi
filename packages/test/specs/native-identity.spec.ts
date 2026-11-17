// JSC tells a constructor from a plain function, and one native from another, by comparing native
// function pointers. A release link that folds identical functions (link.exe /OPT:ICF) made `BigInt`
// and `Symbol` non-constructors; this runs the engine's natives in a packaged app on the bare host
// `sk build` would find. Point SK_HOST at a release host to check that build's link.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { join } from "node:path";
import { launch, packagedExe, REPO, type SkApp } from "../src/index";

const BUILD_T = 20 * 60_000;

let app: SkApp | null = null;
afterAll(() => app?.close());

describe("native function identity", () => {
  test(
    "constructors construct, non-constructors refuse, and distinct natives stay distinct",
    async () => {
      const packed = await packagedExe({
        app: join(REPO, "packages", "test", "fixtures", "native-identity"),
        label: "native-identity",
      });
      app = await launch({
        app: "native-identity",
        exe: packed.exe,
        packaged: packed.name,
        label: "native-identity",
        launchTimeout: BUILD_T,
      });
      const probe = (await app.eval("globalThis.__SK_TEST_NATIVE_IDENTITY__")) as {
        checked: number;
        failures: string[];
      };
      bunExpect(probe.failures).toEqual([]);
      bunExpect(probe.checked).toBeGreaterThan(100);
    },
    BUILD_T,
  );
});
