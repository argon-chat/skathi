// The runtime protocol id (cli/protocol.ts). sk-app's build.rs hashes the same names; packages/test
// compares the two on every launch of the checkout's dev host.

import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { protocolOf } from "../cli/protocol";

const root = mkdtempSync(join(tmpdir(), "sk-protocol-"));
afterAll(() => rmSync(root, { recursive: true, force: true }));

test("the protocol id moves with the set of __sk_ names and nothing else", () => {
  const src = join(root, "src");
  mkdirSync(join(src, "runtime"), { recursive: true });
  writeFileSync(join(src, "protocol.d.ts"), "declare function __sk_intern(name: string): number;\n");
  writeFileSync(join(src, "runtime", "a.ts"), "__sk_intern('x'); globalThis.__sk_log?.('y');\n");
  const base = protocolOf(src);

  writeFileSync(join(src, "runtime", "a.ts"), "// reworded\n__sk_log('y');\n\n__sk_intern('z');\n");
  expect(protocolOf(src)).toBe(base);

  writeFileSync(join(src, "runtime", "b.ts"), "my__sk_thing(); x.__sk_intern;\n");
  expect(protocolOf(src)).toBe(base);

  writeFileSync(join(src, "runtime", "b.ts"), "__sk_text_control_install(proto);\n");
  expect(protocolOf(src)).not.toBe(base);
});
