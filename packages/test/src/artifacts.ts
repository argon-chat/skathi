// `bun test` has no artifacts directory and no reporter hook to attach one to, so the only channel
// from a failed assertion back to a human is the error MESSAGE.

import { existsSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { REPO } from "./dist";

export const ARTIFACTS_DIR = join(REPO, ".sk-test-artifacts");

let seq = 0;

function prepare() {
  rmSync(ARTIFACTS_DIR, { recursive: true, force: true });
  mkdirSync(ARTIFACTS_DIR, { recursive: true });
  writeFileSync(join(ARTIFACTS_DIR, ".gitignore"), "*\n");
  writeFileSync(
    join(ARTIFACTS_DIR, "README.txt"),
    "Screenshots and log dumps @skathi-native/test wrote when an assertion failed.\n" +
      "Wiped at the start of every `bun test` run.\n",
  );
}

prepare();

/** A fresh, collision-free path inside the artifacts directory. */
export function artifactPath(label: string, ext: string): string {
  const slug =
    label
      .replace(/[^a-zA-Z0-9._-]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 60) || "artifact";
  return join(ARTIFACTS_DIR, `${String(++seq).padStart(3, "0")}-${slug}.${ext}`);
}

export function writeArtifact(label: string, ext: string, body: string): string {
  const path = artifactPath(label, ext);
  writeFileSync(path, body);
  return path;
}

export const artifactsExist = () => existsSync(ARTIFACTS_DIR);
