// Templates that live in github.com/argon-chat/skathi-templates. A Skathi checkout has them in its
// templates/ submodule; an installed CLI downloads the commit it was released with, which the release
// writes into gallery.json beside package.json.

import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { ConfigError } from "./config";
import { cacheRoot } from "./host";

export const GALLERY_REPO = "argon-chat/skathi-templates";

const CHECKOUT = resolve(import.meta.dir, "../../../templates");
const PIN = resolve(import.meta.dir, "../gallery.json");

function pinnedCommit(): string | null {
  try {
    const commit: unknown = JSON.parse(readFileSync(PIN, "utf8")).commit;
    return typeof commit === "string" && /^[0-9a-f]{40}$/.test(commit) ? commit : null;
  } catch {
    return null;
  }
}

const cachedGallery = (commit: string) => join(dirname(cacheRoot()), "templates", commit);

/** Where the template's files are, or null when they still have to be downloaded. */
export function galleryDir(template: string): string | null {
  const local = join(CHECKOUT, template);
  if (existsSync(local)) return local;
  const commit = pinnedCommit();
  const cached = commit ? join(cachedGallery(commit), template) : null;
  return cached && existsSync(cached) ? cached : null;
}

/** Downloads the pinned templates unless they are already here. */
export async function fetchGallery(template: string): Promise<string> {
  const ready = galleryDir(template);
  if (ready) return ready;
  const commit = pinnedCommit();
  if (!commit) {
    throw new ConfigError(
      `sk create: the ${template} template comes from github.com/${GALLERY_REPO}; in a Skathi checkout run ` +
        "`git submodule update --init templates`",
    );
  }
  const url = `https://codeload.github.com/${GALLERY_REPO}/tar.gz/${commit}`;
  const res = await fetch(url, { headers: { "User-Agent": "skathi-cli" } });
  if (!res.ok) throw new ConfigError(`sk create: downloading the ${template} template from ${url}: HTTP ${res.status}`);
  const files = untar(Bun.gunzipSync(new Uint8Array(await res.arrayBuffer())));

  const root = cachedGallery(commit);
  const part = `${root}.part-${process.pid}`;
  rmSync(part, { recursive: true, force: true });
  for (const [path, bytes] of files) {
    const rel = path.split("/").slice(1).join("/");
    if (!rel) continue;
    const dest = join(part, rel);
    mkdirSync(dirname(dest), { recursive: true });
    writeFileSync(dest, bytes);
  }
  rmSync(root, { recursive: true, force: true });
  renameSync(part, root);

  const dir = galleryDir(template);
  if (!dir) throw new ConfigError(`sk create: github.com/${GALLERY_REPO} at ${commit} has no ${template} template`);
  return dir;
}

/** The regular files of a tar archive, by path. */
export function untar(tar: Uint8Array): Map<string, Uint8Array> {
  const decoder = new TextDecoder();
  const text = (from: number, length: number) => {
    const field = tar.subarray(from, from + length);
    const end = field.indexOf(0);
    return decoder.decode(end < 0 ? field : field.subarray(0, end));
  };
  const out = new Map<string, Uint8Array>();
  let longName: string | null = null;
  for (let at = 0; at + 512 <= tar.length;) {
    const name = text(at, 100);
    if (!name) break;
    const size = Number.parseInt(text(at + 124, 12).trim() || "0", 8);
    const type = tar[at + 156] ?? 0;
    const prefix = text(at + 345, 155);
    const body = tar.subarray(at + 512, at + 512 + size);
    at += 512 + Math.ceil(size / 512) * 512;

    if (type === 0x78 /* x: pax */) {
      longName = /(?:^|\n)\d+ path=([^\n]*)\n/.exec(decoder.decode(body))?.[1] ?? null;
      continue;
    }
    if (type === 0x4c /* L: GNU long name */) {
      longName = decoder.decode(body).replace(/\0+$/, "");
      continue;
    }
    const path = longName ?? (prefix ? `${prefix}/${name}` : name);
    longName = null;
    if (type !== 0x30 && type !== 0) continue;
    if (path.split("/").some((part) => part === ".." || part === "")) {
      if (!path.endsWith("/")) throw new ConfigError(`the template archive has an unsafe path: ${path}`);
      continue;
    }
    out.set(path, body);
  }
  return out;
}
