import { expect, test } from "bun:test";
import { compat } from "../src/data/css/bcd";
import { entries, entriesBySlug } from "../src/data/css/entries";
import { catalogPath, nativeProperties } from "../src/data/css/native";

test("a new catalog declaration picks up MDN metadata without registering its slug", () => {
  const entry = entriesBySlug.get("width")!;
  const data = {
    css: {
      properties: { "future-property": { __compat: { mdn_url: "/future", status: { experimental: true } } } },
      types: {
        length: {
          __compat: { mdn_url: "/length", status: { experimental: true } },
          xy: { __compat: { status: { deprecated: true } } },
        },
      },
    },
  };
  expect(compat({ ...entry, slug: "future-property", name: "future-property" }, data)).toEqual({
    key: "css.properties.future-property",
    mdn: "/future",
    experimental: true,
  });
  expect(compat({ ...entry, slug: "unit-xy", name: "xy", kind: "unit" }, data)).toEqual({
    key: "css.types.length.xy",
    mdn: "/length",
    deprecated: true,
  });
});

test("every native CSS declaration has a reference page with the mapper's sample values", () => {
  const properties = new Map(
    entries.filter((e) => e.kind === "property" || e.kind === "shorthand").map((e) => [e.name, e]),
  );
  for (const native of nativeProperties.values()) {
    const entry = properties.get(native.name);
    expect(entry, native.name).toBeDefined();
    const samples = new Set(entry?.values?.map((sample) => sample.value));
    for (const value of [...native.applied, ...native.refused])
      expect(samples.has(value), `${native.name}: ${value}`).toBe(true);
  }
});

test("generated declarations distinguish ignored, dropped and implemented values without guessing release history", () => {
  const width = entriesBySlug.get("border-top-width");
  expect(width?.source).toBe(catalogPath);
  expect(width?.since).toBeUndefined();
  expect(width?.values).toContainEqual({ value: "2px", status: "supported" });
  expect(entriesBySlug.get("text-size-adjust")?.values?.[0]?.note).toContain("without a visible effect");
  expect(entriesBySlug.get("webkit-touch-callout")?.summary).toContain("silently drops");
  expect(entriesBySlug.get("box-align")?.status).toBe("unsupported");
  expect(entriesBySlug.get("perspective")?.status).toBe("planned");
});

test("native support, aliases and notes coexist with curated value history and platform restrictions", () => {
  const width = entriesBySlug.get("width");
  expect(width?.status).toBe("partial");
  expect(width?.values?.find((sample) => sample.value === "stretch")?.status).toBe("unsupported");
  expect(width?.values?.find((sample) => sample.value === "calc-size()")?.since).toBe("0.4.0");
  expect(width?.values?.find((sample) => sample.value.startsWith("calc-size(fit-content"))?.since).toBe("0.4.0");
  expect(entriesBySlug.get("align-self")?.aliases).toContain("-webkit-align-self");
  expect(entriesBySlug.get("font-dilation")?.platforms).toEqual({
    windows: "supported",
    macos: "not-planned",
    linux: "partial",
  });
});
