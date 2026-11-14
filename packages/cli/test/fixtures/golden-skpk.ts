// The inputs `golden.skpk` (beside this file) is written from.
//
// ONE fixture, read from both sides: `test/skpk.test.ts` asserts the TS writer
// still produces exactly these bytes, and `crates/sk-payload`'s tests parse the
// same file and assert what they find. A format change that only one side
// learned about fails on the other.
//
// Regenerate (only when the FORMAT changes on purpose — and then the Rust
// expectations change in the same commit):
//
//   SK_UPDATE_GOLDEN=1 bun test packages/cli/test/skpk.test.ts

import { KIND, NAMES, type SkpkInput } from "../../cli/skpk";

const enc = new TextEncoder();

export const GOLDEN_SKEW = 100;

export const GOLDEN_META = {
  format: 1,
  name: "golden",
  productName: "Golden",
  version: "1.2.3",
  skathi: "0.0.1",
  host: { version: "0.0.1", variant: "standard" },
  entry: "src/main.ts",
  assets: 4,
};

export function goldenInputs(): SkpkInput[] {
  return [
    // Deliberately NOT in sorted order: the writer sorts.
    { name: "assets/hello.txt", kind: KIND.ASSET, data: enc.encode("hello\n") },
    { name: NAMES.SCRIPT, kind: KIND.SCRIPT, data: enc.encode("globalThis.__golden = 42;\n") },
    { name: "assets/Logo.PNG", kind: KIND.ASSET, data: new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) },
    { name: NAMES.META, kind: KIND.META, data: enc.encode(JSON.stringify(GOLDEN_META)) },
    { name: "assets/dir/data.bin", kind: KIND.ASSET, data: Uint8Array.from({ length: 256 }, (_, i) => i) },
    { name: NAMES.WINDOW, kind: KIND.WINDOW, data: enc.encode('{"title":"Golden","width":640}') },
    { name: "assets/ünïcode.txt", kind: KIND.ASSET, data: enc.encode("unicode\n") },
  ];
}
