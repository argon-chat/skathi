// Photographs each blog post's link-preview card into public/og/blog/<id>.png.
//   bun scripts/og.ts [--url http://localhost:4321]
// The cards are served by `astro dev` only, so start it first. Needs Chrome or Edge; set BROWSER to
// its path if it is not in a usual place.
import { existsSync, mkdirSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";

const { values } = parseArgs({ options: { url: { type: "string", default: "http://localhost:4321" } } });
const root = join(import.meta.dir, "..");
const out = join(root, "public", "og", "blog");

const browser = [
  process.env["BROWSER"],
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
  "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  "/usr/bin/google-chrome",
  "/usr/bin/chromium",
].find((path) => path && existsSync(path));
if (!browser) throw new Error("No Chrome or Edge found. Set BROWSER to the browser's executable.");

const ids = readdirSync(join(root, "blog"))
  .filter((file) => /\.mdx?$/.test(file))
  .map((file) => file.replace(/\.mdx?$/, ""));
mkdirSync(out, { recursive: true });

for (const id of ids) {
  const page = `${values.url}/og/blog/${id}/`;
  const response = await fetch(page).catch(() => undefined);
  if (!response?.ok) throw new Error(`${page} did not load. Is \`astro dev\` running at ${values.url}?`);
  const file = join(out, `${id}.png`);
  const run = Bun.spawnSync([
    browser,
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    "--force-device-scale-factor=1",
    "--window-size=1200,630",
    "--virtual-time-budget=5000",
    `--screenshot=${file}`,
    page,
  ]);
  if (!existsSync(file)) throw new Error(`The browser wrote no screenshot for ${id}: ${run.stderr.toString()}`);
  console.log(`og/blog/${id}.png`);
}
