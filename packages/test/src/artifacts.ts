// `bun test` has no artifacts directory and no reporter hook to attach one to, so the only channel
// from a failed assertion back to a human is the error MESSAGE.

import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { REPO } from "./dist";

export const ARTIFACTS_DIR = join(REPO, ".sk-test-artifacts");

let seq = 0;

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return (e as NodeJS.ErrnoException).code === "EPERM";
  }
}

/** Clears what finished runs left; a run going on beside this one keeps its files. */
function prepare() {
  mkdirSync(ARTIFACTS_DIR, { recursive: true });
  for (const name of readdirSync(ARTIFACTS_DIR)) {
    if (name === ".gitignore" || name === "README.txt") continue;
    const owner = /^r(\d+)-/.exec(name);
    if (owner && Number(owner[1]) !== process.pid && alive(Number(owner[1]))) continue;
    rmSync(join(ARTIFACTS_DIR, name), { recursive: true, force: true });
  }
  writeFileSync(join(ARTIFACTS_DIR, ".gitignore"), "*\n");
  writeFileSync(
    join(ARTIFACTS_DIR, "README.txt"),
    "Screenshots and log dumps @skathi-native/test wrote when an assertion failed.\n" +
      "Each `bun test` run clears what finished runs left here.\n",
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
  return join(ARTIFACTS_DIR, `r${process.pid}-${String(++seq).padStart(3, "0")}-${slug}.${ext}`);
}

export function writeArtifact(label: string, ext: string, body: string): string {
  const path = artifactPath(label, ext);
  writeFileSync(path, body);
  return path;
}

export const artifactsExist = () => existsSync(ARTIFACTS_DIR);
