// `"intl"` in sk.config.json, on a host that links only the root ICU locale: "full" ships icu.skpk beside the
// exe and every locale formats as in a browser; "invariant" ships nothing and Intl answers from the root
// locale without throwing. Both are packaged apps (`sk build`) on the same root-only host.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { cpSync, existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { featureExe, launch, MACOS, packagedExe, REPO, WORK_DIR, type SkApp } from "../src/index";

const BUILD_T = 20 * 60_000;

interface Probe {
  results: Record<string, unknown>;
  errors: string[];
}

/** The workload `sk build` would stamp when it links only the root locale, else one built so. */
async function rootHost(): Promise<string> {
  const { resolveHost, HostError } = await import("../../cli/cli/host");
  try {
    const host = resolveHost({ variant: "standard" });
    if (host.info.icu?.linked === "root") return host.exe;
  } catch (e) {
    if (!(e instanceof HostError)) throw e;
  }
  return featureExe({ features: ["prod", "icu-root"] });
}

function appWith(intl: string | undefined): string {
  const dir = join(WORK_DIR, "intl", `app-${intl ?? "default"}-${process.pid}`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  cpSync(join(REPO, "packages", "test", "fixtures", "intl", "src"), join(dir, "src"), { recursive: true });
  writeFileSync(
    join(dir, "sk.config.json"),
    JSON.stringify({
      name: `sk-test-intl-${intl ?? "default"}`,
      version: "0.0.1",
      assets: [],
      ...(intl ? { intl } : {}),
      window: { title: `Skathi — intl ${intl ?? "default"} (fixture)`, width: 320, height: 200 },
    }),
  );
  return dir;
}

let host: string | undefined;
const apps: SkApp[] = [];
const dirs: string[] = [];
afterAll(() => {
  for (const app of apps) app.close();
  for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
});

async function run(intl: string | undefined) {
  const dir = appWith(intl);
  dirs.push(dir);
  const label = `intl-${intl ?? "default"}`;
  host ??= await rootHost();
  const packed = await packagedExe({ app: dir, host, label });
  const app = await launch({ app: label, exe: packed.exe, packaged: packed.name, label, launchTimeout: BUILD_T });
  apps.push(app);
  const probe = (await app.eval("globalThis.__SK_TEST_INTL__")) as Probe;
  return { packed, probe };
}

// macOS hosts link every ICU locale; none is root-only.
describe.skipIf(MACOS)("intl", () => {
  test(
    "the default ships icu.skpk and every locale formats as in a browser",
    async () => {
      const { packed, probe } = await run(undefined);
      bunExpect(packed.result.icu).not.toBeNull();
      bunExpect(existsSync(join(dirname(packed.built), "icu.skpk"))).toBe(true);
      bunExpect(probe.errors).toEqual([]);
      bunExpect(probe.results).toMatchObject({
        month: "Januar",
        number: "1.234,5",
        sort: "azä",
        region: "Germany",
        list: "a, b und c",
        words: "日本語|の|文章|です",
        upper: "İ",
      });
    },
    BUILD_T,
  );

  test(
    '"invariant" ships no ICU data, answers from the root locale, and nothing throws',
    async () => {
      const { packed, probe } = await run("invariant");
      bunExpect(packed.result.icu).toBeNull();
      bunExpect(existsSync(join(dirname(packed.built), "icu.skpk"))).toBe(false);
      bunExpect(probe.errors).toEqual([]);
      bunExpect(probe.results).toMatchObject({
        month: "M01",
        number: "1,234.5",
        sort: "aäz",
        region: "DE",
        list: "a, b, c",
        words: "日本語の文章です",
        resolved: "de",
        plural: "few",
        upper: "İ",
        normalize: "Dž",
        timeZone: "21",
      });
    },
    BUILD_T,
  );
});
