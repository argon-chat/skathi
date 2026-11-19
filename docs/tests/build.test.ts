import { describe, expect, test } from "bun:test";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { entries } from "../src/data/css/entries";
import type { SearchData } from "../src/lib/search";

const dist = fileURLToPath(new URL("../dist/", import.meta.url));
const read = (path: string): string => readFileSync(join(dist, path), "utf8");
const pages = ["introduction", "quick-start", "concepts", "css", "web-platform", "authoring"];
const pagePath = (id: string): string => `${id}/index.html`;

describe("built documentation (run bun run build first)", () => {
  test("renders every page to HTML and exports the source to agents", () => {
    const index = read("llms.txt");
    const full = read("llms-full.txt");
    for (const id of pages) {
      const html = read(pagePath(id));
      const markdown = read(`${id}.md`);
      expect(html).toContain('<html lang="en"');
      expect([...html.matchAll(/<h1\b/g)]).toHaveLength(1);
      expect(html).toContain(`href="/${id}.md"`);
      expect(markdown).toMatch(/^# /);
      expect(index).toContain(`/${id}.md`);
      expect(full).toContain(markdown.trim());
    }
  });

  test("highlights each requested language with real syntax tokens", () => {
    const html = read("authoring/index.html");
    for (const language of ["slang", "js", "ts", "rust"]) {
      const block = [...html.matchAll(/<pre\b([^>]*)>([\s\S]*?)<\/pre>/g)].find((match) =>
        match[1]?.includes(`data-language="${language}"`),
      );
      expect(block).toBeDefined();
      expect(block?.[2]).toContain("--shiki-dark:");
      expect(block?.[2]).toContain("--shiki-light:");
    }
    expect(html).toContain("astro-island");
    expect(html).toContain("CounterDemo");
  });

  test("rewrites the original relative Markdown links for HTML navigation", () => {
    const html = read("web-platform/index.html");
    expect(html).toContain('href="/css/">CSS coverage</a>');
    expect(html).not.toContain('href="css.md"');
    expect(read("web-platform.md")).toContain("[CSS coverage](css.md)");
  });

  test("provides metadata, an accurate sitemap, and the search index", () => {
    const sitemap = read("sitemap-0.xml");
    const home = read("index.html");
    const canonical = /<link rel="canonical" href="([^"]+)"/.exec(home)?.[1];
    expect(canonical).toBeDefined();
    const origin = new URL(canonical ?? "http://localhost:4321/").origin;
    for (const id of pages) {
      expect(sitemap).toContain(`<loc>${origin}/${id}/</loc>`);
      expect(read(pagePath(id))).toContain("application/ld+json");
      expect(read(pagePath(id))).toContain('property="og:image"');
    }
    expect(sitemap).toContain(`<loc>${origin}/</loc>`);
    expect(home).toContain('property="og:image"');
    expect(sitemap).not.toContain("404");
    expect(read("robots.txt")).toContain(`${origin}/sitemap-index.xml`);
    expect(read("llms.txt")).toContain(`${origin}/introduction.md`);
    expect(existsSync(join(dist, "social.png"))).toBe(true);
    expect(read("404.html")).toContain('name="robots" content="noindex"');
  });

  test("distinguishes CSS properties from media queries in search and in the Markdown reference", () => {
    const index = JSON.parse(read("search-index.json")) as SearchData;
    const titles = new Map(index.records.map(([, title, , url]) => [url, title]));
    const names = new Set<string>();
    for (const entry of entries) {
      expect(names.has(entry.name), entry.name).toBe(false);
      names.add(entry.name);
      if (entry.kind !== "media-feature") continue;
      expect(titles.get(`/css/${entry.slug}/`)).toBe(entry.name);
      expect(read(`css/${entry.slug}.md`).split("\n")[0]).toBe(`# ${entry.name}`);
    }
    expect(titles.get("/css/aspect-ratio/")).toBe("aspect-ratio");
    expect(titles.get("/css/media-aspect-ratio/")).toBe("@media (aspect-ratio)");
    expect(read("css/aspect-ratio.md")).toContain("Status in Skathi: Supported since 0.2.0");
  });

  test("a CSS entry with a working value has an available status", () => {
    for (const entry of entries) {
      if (entry.values?.some((v) => v.status === "supported" || v.status === "partial")) {
        expect(["supported", "partial"], entry.name).toContain(entry.status);
      }
    }
  });

  test("has no broken internal page, asset, or heading links", async () => {
    for await (const file of new Bun.Glob("**/*.html").scan({ cwd: dist })) {
      const html = read(file);
      const base = new URL(file.replace(/index\.html$/, ""), "https://docs.test/");
      for (const match of html.matchAll(/(?:href|src)="([^"\s]+)"/g)) {
        const value = match[1];
        if (!value || /^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(value)) continue;
        const target = new URL(value, base);
        const path = decodeURIComponent(target.pathname).replace(/^\//, "");
        const resolved = join(dist, path.endsWith("/") || !path ? `${path}index.html` : path);
        expect(existsSync(resolved), `${file}: ${value}`).toBe(true);
        if (target.hash && resolved.endsWith(".html")) {
          const id = decodeURIComponent(target.hash.slice(1));
          expect(readFileSync(resolved, "utf8"), `${file}: ${value}`).toContain(`id="${id}"`);
        }
      }
    }
  });

  test("points every search result at a built page and heading", () => {
    const index = JSON.parse(read("search-index.json")) as SearchData;
    const pages = new Map<string, string>();
    for (const [, , , url] of index.records) {
      const [path = "", hash] = url.split("#");
      const file = `${path.replace(/^\//, "")}index.html`;
      if (!pages.has(file)) pages.set(file, existsSync(join(dist, file)) ? read(file) : "");
      expect(pages.get(file), url).not.toBe("");
      if (hash) expect(pages.get(file), url).toContain(`id="${hash}"`);
    }
  });
});
