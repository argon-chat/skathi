// The engine built-ins `sk check` knows, from the pinned native-AOT builtin table.
//
// Usage: bun scripts/aot/check-builtins.ts [--table <dir with aot_builtin_table.json>]
// Writes packages/cli/check/builtins.json. Only engine-origin (ECMAScript) entries are kept: the
// table's host entries describe Bun's own APIs, which a Skathi host does not have.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { loadPin } from "./toolchain";

const SDK = resolve(import.meta.dir, "../..");
const PIN = loadPin(join(import.meta.dir, "toolchain.lock.json"));
const OUT = join(SDK, "packages/cli/check/builtins.json");

interface Entry {
  path: string;
  kind: string;
  origin: string;
  mutability: string;
  flags: string[];
}

function tableDir(): string {
  const at = process.argv.indexOf("--table");
  if (at >= 0) {
    const dir = process.argv[at + 1];
    if (!dir) throw Error("--table needs a directory");
    return resolve(dir);
  }
  const name = `table-${PIN.table.id}`;
  for (const root of [SDK, resolve(SDK, "../..")]) {
    const dir = join(root, ".tools", "native-aot", name);
    if (existsSync(join(dir, "aot_builtin_table.json"))) return dir;
  }
  throw Error(`No ${name}/aot_builtin_table.json under .tools/native-aot; pass --table <dir>`);
}

const file = join(tableDir(), "aot_builtin_table.json");
const table = JSON.parse(readFileSync(file, "utf8")) as { entries: Entry[] };
const engine = table.entries.filter((e) => e.origin === "Engine" && !e.path.includes("%"));
const paths = (pick: (e: Entry) => boolean) => [...new Set(engine.filter(pick).map((e) => e.path))].sort();

const data = {
  table: PIN.table.id,
  engine: PIN.engine.revision,
  locked: paths((e) => e.mutability === "AlwaysLocked"),
  replaceable: paths((e) => e.mutability !== "AlwaysLocked"),
  lockedObjects: paths((e) => e.flags.includes("IsLockedObject")),
  objects: paths((e) => e.kind === "Prototype" || e.kind === "Constructor" || e.kind === "Namespace"),
};
writeFileSync(OUT, JSON.stringify(data, null, 2) + "\n");
console.log(`${OUT}: ${data.locked.length} locked, ${data.replaceable.length} replaceable built-ins`);
