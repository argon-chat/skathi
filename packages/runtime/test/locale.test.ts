import { afterEach, expect, test } from "bun:test";

const globals = globalThis as Record<string, unknown>;
const HOST = ["__sk_locale_state", "__sk_locale_set_language", "__sk_locale_define", "__sk_locale_t"];
let sequence = 0;
const load = async (): Promise<typeof import("../src/locale")> => import(`../src/locale.ts?locale-case=${++sequence}`);
afterEach(() => {
  for (const name of HOST) delete globals[name];
});

test("appLocale is the host's: state, override, define and t", async () => {
  const calls: string[] = [];
  let override: string | null = null;
  globals.__sk_locale_state = () =>
    JSON.stringify({ language: override ?? "ru", override, available: ["en", "ru"], preferences: ["ru-RU"] });
  globals.__sk_locale_set_language = (tag: string) => {
    if (tag === "!") return '"!" is not a BCP 47 language tag';
    override = tag || null;
    return "";
  };
  globals.__sk_locale_define = (json: string) => {
    calls.push(json);
    return json.includes("??") ? "bad tag" : "";
  };
  globals.__sk_locale_t = (key: string, args: string) => `${key}|${args}`;
  const { appLocale } = await load();

  expect(appLocale.resolved).toBe("ru");
  expect(appLocale.language).toBeNull();
  expect(appLocale.available).toEqual(["en", "ru"]);
  appLocale.language = "de";
  expect([appLocale.language, appLocale.resolved]).toEqual(["de", "de"]);
  expect(() => (appLocale.language = "!")).toThrow(RangeError);
  appLocale.language = null;
  expect(appLocale.language).toBeNull();

  appLocale.define({ de: { "app.hi": "Hallo {name}" } });
  expect(calls).toEqual(['{"de":{"app.hi":"Hallo {name}"}}']);
  expect(() => appLocale.define({ "??": {} })).toThrow(TypeError);
  expect(appLocale.t("app.hi", { name: "Ada", n: 2 })).toBe('app.hi|{"name":"Ada","n":"2"}');
});

test("without a host appLocale still defines and looks up", async () => {
  const { appLocale } = await load();
  appLocale.define({ en: { hi: "Hi {name}", only: "en" }, de: { hi: "Hallo {name}" } });
  appLocale.language = "de-AT";
  expect(appLocale.resolved).toBe("de");
  expect(appLocale.t("hi", { name: "Ada" })).toBe("Hallo Ada");
  expect(appLocale.t("only")).toBe("en");
  expect(appLocale.t("missing")).toBe("missing");
  expect(appLocale.t("hi", {})).toBe("Hallo {name}");
});
