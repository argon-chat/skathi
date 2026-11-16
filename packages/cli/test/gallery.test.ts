import { expect, test } from "bun:test";
import { ConfigError } from "../cli/config";
import { untar } from "../cli/gallery";

function entry(name: string, body: string, type = "0", prefix = ""): Uint8Array {
  const data = new TextEncoder().encode(body);
  const header = new Uint8Array(512);
  const put = (at: number, s: string) => header.set(new TextEncoder().encode(s), at);
  put(0, name);
  put(124, data.length.toString(8).padStart(11, "0"));
  put(156, type);
  put(345, prefix);
  const out = new Uint8Array(512 + Math.ceil(data.length / 512) * 512);
  out.set(header);
  out.set(data, 512);
  return out;
}

const tar = (...parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0) + 1024);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
};

const text = (files: Map<string, Uint8Array>) =>
  Object.fromEntries([...files].map(([k, v]) => [k, new TextDecoder().decode(v)]));

test("untar reads regular files under pax and ustar long names, and skips the rest", () => {
  const long = `repo-abc/player/${"deep/".repeat(30)}file.ts`;
  const files = untar(
    tar(
      entry("pax_global_header", "52 comment=abc\n", "g"),
      entry("repo-abc/", "", "5"),
      entry("repo-abc/player/main.ts", "mount();"),
      entry("PaxHeader", `${long.length + 9 + String(long.length + 9).length} path=${long}\n`, "x"),
      entry("ignored-name", "deep"),
      entry("icon.png", "png", "0", "repo-abc/player"),
    ),
  );
  expect(text(files)).toEqual({
    "repo-abc/player/main.ts": "mount();",
    [long]: "deep",
    "repo-abc/player/icon.png": "png",
  });
});

test("untar refuses a path that climbs out of the archive", () => {
  expect(() => untar(tar(entry("repo-abc/../../evil.ts", "x")))).toThrow(ConfigError);
});
