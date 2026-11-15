import { expect, test } from "bun:test";

let sequence = 0;
const load = async (): Promise<typeof import("../src/locale")> => import(`../src/locale.ts?locale-case=${++sequence}`);

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
