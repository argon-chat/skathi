import { createApp, h } from "@skathi-native/runtime";

const date = new Date(Date.UTC(2024, 0, 15, 12, 30));
const probes: Record<string, () => unknown> = {
  month: () => new Intl.DateTimeFormat("de", { month: "long", timeZone: "UTC" }).format(date),
  dateStyle: () => new Intl.DateTimeFormat("de", { dateStyle: "long", timeZone: "UTC" }).format(date),
  resolved: () => new Intl.DateTimeFormat("de").resolvedOptions().locale,
  toLocaleString: () => date.toLocaleString("ja-JP", { timeZone: "UTC" }),
  number: () => (1234.5).toLocaleString("de"),
  currency: () => new Intl.NumberFormat("fr", { style: "currency", currency: "EUR" }).format(1234.5),
  sort: () => ["z", "ä", "a"].sort(new Intl.Collator("sv").compare).join(""),
  localeCompare: () => "ä".localeCompare("z", "sv"),
  region: () => new Intl.DisplayNames("en", { type: "region" }).of("DE"),
  plural: () => new Intl.PluralRules("ru").select(3),
  list: () => new Intl.ListFormat("de").format(["a", "b", "c"]),
  relative: () => new Intl.RelativeTimeFormat("de").format(-1, "day"),
  words: () =>
    [...new Intl.Segmenter("ja", { granularity: "word" }).segment("日本語の文章です")].map((s) => s.segment).join("|"),
  thai: () =>
    [...new Intl.Segmenter("th", { granularity: "word" }).segment("ภาษาไทยง่าย")].map((s) => s.segment).join("|"),
  graphemes: () => [...new Intl.Segmenter("en", { granularity: "grapheme" }).segment("é👍🏽")].length,
  upper: () => "i".toLocaleUpperCase("tr"),
  normalize: () => "ǅ".normalize("NFKC"),
  timeZone: () =>
    new Intl.DateTimeFormat("en", { timeZone: "Asia/Tokyo", hour: "numeric", hour12: false }).format(date),
};

const results: Record<string, unknown> = {};
const errors: string[] = [];
for (const [name, probe] of Object.entries(probes)) {
  try {
    results[name] = probe();
  } catch (e) {
    errors.push(`${name}: ${String(e)}`);
  }
}
(globalThis as unknown as { __SK_TEST_INTL__: unknown }).__SK_TEST_INTL__ = { results, errors };

createApp({ render: () => h("div", { id: "intl" }, String(results.month)) }).mount("#app");
