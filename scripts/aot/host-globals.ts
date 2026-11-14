// The global names the pinned builtin table gives host (Bun) implementations, which the
// opaque-host transform keeps opaque so the frontend never assumes Bun's layouts for them.
//
// Usage: bun scripts/aot/host-globals.ts [--table <dir with aot_builtin_table.json>] [--check]
// Writes packages/cli/aot/host-globals.json; --check fails instead when it differs.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { hostGlobalsFromTable, type BuiltinEntry } from "../../packages/cli/aot/transform";
import { loadPin, verifyFile } from "./toolchain";

const SDK = resolve(import.meta.dir, "../..");
const PIN = loadPin();
const OUT = join(SDK, "packages/cli/aot/host-globals.json");

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

const dump = join(tableDir(), "aot_builtin_table.json");
await verifyFile(dump, PIN.table.dumpSha256);
const table = JSON.parse(readFileSync(dump, "utf8")) as { entries: BuiltinEntry[] };
const text = JSON.stringify({ table: PIN.table.id, names: hostGlobalsFromTable(table.entries) }, null, 2) + "\n";
if (process.argv.includes("--check")) {
  if (!existsSync(OUT) || readFileSync(OUT, "utf8").replace(/\r\n/g, "\n") !== text)
    throw Error(`${OUT} differs from the pinned table; run bun scripts/aot/host-globals.ts`);
  console.log(`${OUT} matches table ${PIN.table.id}`);
} else {
  writeFileSync(OUT, text);
  console.log(`${OUT}: ${JSON.parse(text).names.length} host globals`);
}
