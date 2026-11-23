// Card rows on the landing end flush, no card grid bypasses CardRow, and every section intro is a
// SectionIntro that uses the whole row.
// Usage: bun scripts/check-layout.ts [--url http://127.0.0.1:4321/]
// Without --url it serves dist/ (run `bun run build` first). The browser is CHROMIUM_PATH, a
// Playwright download, or an installed Chrome or Edge.
import { existsSync, readdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { extname, join, resolve } from "node:path";
import { chromium, type Browser } from "playwright-core";

const widths = [1440, 1100, 390] as const;
const themes = ["dark", "light"] as const;
const tolerance = 1;

const urlFlag = process.argv.indexOf("--url");
const given = urlFlag >= 0 ? process.argv[urlFlag + 1] : undefined;

function playwrightBrowsers(): string[] {
  const roots = [
    process.env["PLAYWRIGHT_BROWSERS_PATH"],
    process.env["LOCALAPPDATA"] && join(process.env["LOCALAPPDATA"], "ms-playwright"),
    join(homedir(), ".cache", "ms-playwright"),
    join(homedir(), "Library", "Caches", "ms-playwright"),
  ].filter((root): root is string => !!root && existsSync(root));
  const names = [
    ["chrome-headless-shell-win64", "chrome-headless-shell.exe"],
    ["chrome-headless-shell-linux64", "chrome-headless-shell"],
    ["chrome-headless-shell-mac-arm64", "chrome-headless-shell"],
    ["chrome-headless-shell-mac-x64", "chrome-headless-shell"],
    ["chrome-win", "chrome.exe"],
    ["chrome-win64", "chrome.exe"],
    ["chrome-linux", "chrome"],
  ];
  const found: string[] = [];
  for (const root of roots) {
    const builds = readdirSync(root)
      .filter((name) => name.startsWith("chromium"))
      .sort()
      .reverse();
    for (const build of builds)
      for (const parts of names) {
        const exe = join(root, build, ...parts);
        if (existsSync(exe)) found.push(exe);
      }
  }
  return found;
}

async function launch(): Promise<Browser> {
  const paths = [process.env["CHROMIUM_PATH"], ...playwrightBrowsers()].filter((p): p is string => !!p);
  for (const executablePath of paths) {
    try {
      return await chromium.launch({ executablePath, headless: true });
    } catch {}
  }
  for (const channel of ["chrome", "msedge"]) {
    try {
      return await chromium.launch({ channel, headless: true });
    } catch {}
  }
  throw new Error(
    "no Chromium found: set CHROMIUM_PATH to a Chrome or Chromium executable, or run `bunx playwright install chromium`",
  );
}

function serveDist(): { url: string; stop: () => void } {
  const root = resolve(import.meta.dir, "..", "dist");
  if (!existsSync(join(root, "index.html")))
    throw new Error("dist/ has no build: run `bun run build` first, or pass --url");
  const server = Bun.serve({
    port: 0,
    hostname: "127.0.0.1",
    fetch(request) {
      const path = decodeURIComponent(new URL(request.url).pathname);
      let file = join(root, path);
      if (!file.startsWith(root)) return new Response("forbidden", { status: 403 });
      if (existsSync(file) && statSync(file).isDirectory()) file = join(file, "index.html");
      else if (!extname(file) && existsSync(`${file}/index.html`)) file = join(file, "index.html");
      return existsSync(file) ? new Response(Bun.file(file)) : new Response("not found", { status: 404 });
    },
  });
  return { url: `http://127.0.0.1:${server.port}/`, stop: () => void server.stop(true) };
}

interface Finding {
  kind: "ragged" | "hand-rolled" | "intro";
  where: string;
  detail: string;
}

function audit(tolerance: number): Finding[] {
  const describe = (el: Element): string => {
    const section = el.closest("section");
    const name = (node: Element) => node.tagName.toLowerCase() + [...node.classList].map((c) => `.${c}`).join("");
    return section && section !== el ? `${name(section)} ${name(el)}` : name(el);
  };
  const painted = (el: Element): boolean => {
    if (["IMG", "VIDEO", "SVG", "CANVAS", "PICTURE", "PRE"].includes(el.tagName.toUpperCase())) return true;
    const style = getComputedStyle(el);
    const bordered =
      parseFloat(style.borderTopWidth) > 0 &&
      parseFloat(style.borderBottomWidth) > 0 &&
      style.borderTopStyle !== "none";
    const filled = !/rgba\(.*,\s*0\)|transparent/.test(style.backgroundColor) || style.backgroundImage !== "none";
    return bordered || filled;
  };
  const shown = (el: Element): boolean => {
    const style = getComputedStyle(el);
    const box = el.getBoundingClientRect();
    return style.display !== "none" && style.visibility !== "hidden" && box.height > 0 && style.position !== "absolute";
  };
  const visualBottom = (el: Element): number => {
    const children = [...el.children].filter(shown);
    const text = [...el.childNodes].some((node) => node.nodeType === Node.TEXT_NODE && node.textContent?.trim());
    if (painted(el) || text || children.length === 0) return el.getBoundingClientRect().bottom;
    return Math.max(...children.map(visualBottom));
  };
  const columns = (el: Element): number =>
    getComputedStyle(el)
      .gridTemplateColumns.split(" ")
      .filter((track) => track !== "").length;
  const card = (el: Element | null): boolean => {
    if (!el) return false;
    const style = getComputedStyle(el);
    return (
      parseFloat(style.borderTopWidth) >= 1 &&
      parseFloat(style.borderBottomWidth) >= 1 &&
      parseFloat(style.borderTopLeftRadius) >= 4
    );
  };

  const findings: Finding[] = [];
  const main = document.querySelector("main");
  if (!main) return [{ kind: "hand-rolled", where: "page", detail: "no <main>" }];

  for (const row of main.querySelectorAll("[data-card-row]")) {
    if (columns(row) < 2) continue;
    const lines = new Map<number, Element[]>();
    for (const child of [...row.children].filter(shown)) {
      const top = Math.round(child.getBoundingClientRect().top);
      const key = [...lines.keys()].find((k) => Math.abs(k - top) <= tolerance) ?? top;
      lines.set(key, [...(lines.get(key) ?? []), child]);
    }
    for (const line of lines.values()) {
      if (line.length < 2) continue;
      const bottoms = line.map(visualBottom);
      const spread = Math.max(...bottoms) - Math.min(...bottoms);
      if (spread > tolerance)
        findings.push({
          kind: "ragged",
          where: describe(row),
          detail: `columns end ${spread.toFixed(1)}px apart: ${line
            .map((child, i) => `${describe(child)} at ${bottoms[i]?.toFixed(1)}`)
            .join(", ")}`,
        });
    }
  }

  for (const el of main.querySelectorAll("*")) {
    if (el.hasAttribute("data-card-row")) continue;
    const display = getComputedStyle(el).display;
    if ((display !== "grid" && display !== "inline-grid") || columns(el) < 2) continue;
    const cards = [...el.children].filter(
      (child) =>
        card(child) || card(child.firstElementChild) || card(child.firstElementChild?.firstElementChild ?? null),
    );
    if (cards.length >= 2)
      findings.push({
        kind: "hand-rolled",
        where: describe(el),
        detail: `a grid of ${cards.length} cards outside CardRow`,
      });
  }

  for (const heading of main.querySelectorAll("section:not(.hero) h2")) {
    if (!heading.closest("[data-intro]"))
      findings.push({ kind: "intro", where: describe(heading), detail: "a section heading outside SectionIntro" });
  }

  // Painted boxes count by their edge, text by its last glyph, so a capped paragraph is caught.
  const visualRight = (el: Element): number => {
    if (painted(el)) return el.getBoundingClientRect().right;
    const text = [...el.childNodes].some((node) => node.nodeType === Node.TEXT_NODE && node.textContent?.trim());
    if (text) {
      const range = document.createRange();
      range.selectNodeContents(el);
      return Math.max(...[...range.getClientRects()].map((rect) => rect.right));
    }
    const children = [...el.children].filter(shown);
    return children.length === 0 ? el.getBoundingClientRect().right : Math.max(...children.map(visualRight));
  };
  const visualTop = (el: Element): number => {
    const first = [...el.children].find(shown);
    return first ? first.getBoundingClientRect().top : el.getBoundingClientRect().top;
  };
  // A ragged right edge of text ends up to about a word short of its column.
  const ragTolerance = 56;
  if (window.innerWidth > 900)
    for (const intro of main.querySelectorAll("[data-intro]")) {
      const variant = intro.getAttribute("data-intro");
      if (variant === "center") continue;
      const right = intro.getBoundingClientRect().right;
      const reach = Math.max(...[...intro.children].filter(shown).map(visualRight));
      if (right - reach > ragTolerance)
        findings.push({
          kind: "intro",
          where: describe(intro),
          detail: `content stops ${(right - reach).toFixed(1)}px short of the row's right edge`,
        });
      if (variant !== "aside") continue;
      const text = intro.querySelector(":scope > .intro-main");
      const side = intro.querySelector(":scope > .intro-side");
      if (!text || !side) continue;
      const tops = Math.abs(visualTop(text) - visualTop(side));
      const bottoms = Math.abs(visualBottom(text) - visualBottom(side));
      if (tops > tolerance || bottoms > tolerance)
        findings.push({
          kind: "intro",
          where: describe(intro),
          detail: `text and aside start ${tops.toFixed(1)}px and end ${bottoms.toFixed(1)}px apart`,
        });
    }
  return findings;
}

const server = given ? undefined : serveDist();
const base = given ?? server?.url ?? "";
const browser = await launch();
let failures = 0;
try {
  for (const theme of themes)
    for (const width of widths) {
      const context = await browser.newContext({
        viewport: { width, height: 900 },
        colorScheme: theme,
        reducedMotion: "reduce",
      });
      await context.addInitScript((value) => {
        try {
          localStorage.setItem("skathi-theme", value);
        } catch {}
      }, theme);
      const page = await context.newPage();
      await page.goto(base, { waitUntil: "load" });
      await page.addStyleTag({
        content: "*, *::before, *::after { animation: none !important; transition: none !important; }",
      });
      await page.evaluate(() => document.fonts.ready.then(() => undefined));
      const findings = await page.evaluate(audit, tolerance);
      for (const finding of findings) {
        failures += 1;
        console.error(`${theme} ${width}px ${finding.kind}: ${finding.where}: ${finding.detail}`);
      }
      await context.close();
    }
} finally {
  await browser.close();
  server?.stop();
}

if (failures > 0) {
  console.error(
    `\n${failures} layout problem(s). Put card rows in <CardRow> and section intros in <SectionIntro>, and balance their columns.`,
  );
  process.exit(1);
}
console.log(`Card rows end flush at ${widths.join(", ")}px in both themes.`);
