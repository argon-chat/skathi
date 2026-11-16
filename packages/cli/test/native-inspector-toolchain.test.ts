import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import {
  canonicalTableReceipt,
  fileDigest,
  loadPin,
  TABLE_FILES,
  verifyCompilerInfo,
  verifyEngine,
  verifyFile,
  verifyTable,
} from "../../../scripts/aot/toolchain";

const pinPath = resolve(import.meta.dir, "../aot/toolchain.lock.json");
const pin = loadPin(pinPath);
function fixture(run: (root: string) => void | Promise<void>): Promise<void> {
  const root = mkdtempSync(join(tmpdir(), "sk-aot-pins-"));
  return Promise.resolve()
    .then(() => run(root))
    .finally(() => rmSync(root, { recursive: true, force: true }));
}
function info() {
  return {
    target: pin.target,
    frontend: pin.frontend.revision,
    engine: pin.engine.revision,
    tableId: pin.table.id,
    compileOptions: `x86_64;references=narrow;image=pic;threads=1;default-options@${pin.engine.revision}`,
  };
}

describe("native inspector build provenance", () => {
  test("table receipt bytes are independent of checkout line endings", () => {
    const receipt = canonicalTableReceipt(pin.frontend.revision, pin.engine.revision);
    expect(Buffer.byteLength(receipt)).toBe(128);
    expect(receipt.split("\r\n")).toEqual([
      "redjs.aot-builtin-table.v1",
      `frontend=${pin.frontend.revision}`,
      `webkit=${pin.engine.revision}`,
      "",
    ]);
    expect(receipt.replaceAll("\r\n", "").includes("\n")).toBe(false);
  });

  test("official frontend and every native compatibility field are pinned", async () => {
    verifyCompilerInfo(info(), pin);
    for (const name of Object.keys(info()))
      expect(() => verifyCompilerInfo({ ...info(), [name]: "stale" }, pin)).toThrow();
    await fixture((root) => {
      for (const change of [
        { frontend: { ...pin.frontend, sourceRepository: "https://example.com/bun.git" } },
        { frontend: { ...pin.frontend, revision: "main" } },
        { engine: { ...pin.engine, archiveSha256: "" } },
        { table: { ...pin.table, id: "not-a-digest" } },
        { target: "aarch64-macos" },
      ]) {
        const path = join(root, "pin.json");
        writeFileSync(path, JSON.stringify({ ...pin, ...change }));
        expect(() => loadPin(path)).toThrow();
      }
    });
  });
  test("streamed artifact verification rejects modified bytes", () =>
    fixture(async (root) => {
      const path = join(root, "artifact");
      const bytes = Buffer.alloc(200_003, 17);
      const expected = createHash("sha256").update(bytes).digest("hex");
      writeFileSync(path, bytes);
      expect(await fileDigest(path)).toBe(expected);
      await verifyFile(path, expected);
      bytes[190_000] = 99;
      writeFileSync(path, bytes);
      await expect(verifyFile(path, expected)).rejects.toThrow();
    }));
  test("engine archive and AOT-enabled installed headers must agree", () =>
    fixture((root) => {
      const redjs = join(root, "vendor/redjs");
      const headers = join(redjs, "prebuilt/bun-webkit/include");
      mkdirSync(headers, { recursive: true });
      writeFileSync(
        join(redjs, "pins.txt"),
        `webkit.version = ${pin.engine.revision}\nwebkit.sha256 = ${pin.engine.archiveSha256}\nwebkit.aot = 1\n`,
      );
      const configuration = join(headers, "cmakeconfig.h");
      writeFileSync(configuration, `// ${pin.engine.revision}\n#define BUN_WEBKIT_ENABLE_AOT 1\n`);
      verifyEngine(root, pin);
      writeFileSync(configuration, `// ${pin.engine.revision}\n#define BUN_WEBKIT_ENABLE_AOT 0\n`);
      expect(() => verifyEngine(root, pin)).toThrow();
      writeFileSync(configuration, "#define BUN_WEBKIT_ENABLE_AOT 1\n");
      expect(() => verifyEngine(root, pin)).toThrow();
    }));
  test("table ID covers every generated native table and the exact engine wire schema", () =>
    fixture(async (root) => {
      const table = join(root, "table"),
        headers = join(root, "vendor/redjs/prebuilt/bun-webkit/include/JavaScriptCore/aot");
      mkdirSync(table);
      mkdirSync(headers, { recursive: true });
      for (const name of TABLE_FILES)
        writeFileSync(
          join(table, name),
          name === ".redjs-aot-table"
            ? `frontend=${pin.frontend.revision}\nwebkit=${pin.engine.revision}\n`
            : `// fixed ${name}\n`,
        );
      const wire = join(headers, "AOTWire.cpp");
      writeFileSync(wire, "// schema v1\n");
      const dump = join(table, "aot_builtin_table.json");
      writeFileSync(dump, '{"entries":[]}\n');
      const hash = createHash("sha256");
      for (const name of TABLE_FILES) hash.update(name).update(readFileSync(join(table, name)));
      hash.update(readFileSync(wire));
      const expected = { ...pin, table: { id: hash.digest("hex"), dumpSha256: await fileDigest(dump) } };
      await verifyTable(table, root, expected);
      writeFileSync(wire, "// different schema\n");
      await expect(verifyTable(table, root, expected)).rejects.toThrow();
      writeFileSync(wire, "// schema v1\n");
      writeFileSync(dump, '{"entries":["changed"]}');
      await expect(verifyTable(table, root, expected)).rejects.toThrow();
    }));
});
