import { existsSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { brotliDecompressSync } from "node:zlib";

interface Span {
  start: number;
  end: number;
}

export type CssNode = Span &
  (
    | { kind: "declaration"; name: string; value: string; important: boolean }
    | { kind: "rule"; selector: string; nodes: CssNode[] }
    | { kind: "at"; name: string; params: string; body: string | null; nodes: CssNode[] }
  );

interface CssDiagnostic extends Span {
  code: string;
  message: string;
  help: string;
}

interface Checker {
  check(length: number): void;
  memory: WebAssembly.Memory;
}

const modulePath = fileURLToPath(new URL("./sk-css.wasm.br", import.meta.url));
let ensured = false;

/** In a repository checkout, builds the checker when it is missing or older than its sources. */
export function ensureCssEngine() {
  if (ensured) return;
  const builder = fileURLToPath(new URL("../../../scripts/css-engine.ts", import.meta.url));
  if (existsSync(builder)) {
    (createRequire(import.meta.url)(builder) as { ensureCssEngine(): void }).ensureCssEngine();
  } else if (!existsSync(modulePath)) {
    throw new Error("This @skathi-native/cli install has no CSS checker (check/sk-css.wasm.br). Reinstall the CLI.");
  }
  ensured = true;
}

function loadEngine() {
  ensureCssEngine();
  const module = new WebAssembly.Module(brotliDecompressSync(readFileSync(modulePath)));
  let input: Uint8Array;
  let result: CssDiagnostic[];
  const decode = (pointer: number, length: number) =>
    new TextDecoder().decode(new Uint8Array(exports.memory.buffer, pointer, length));
  const imports: WebAssembly.Imports = {
    sk: {
      read(pointer: number, length: number) {
        new Uint8Array(exports.memory.buffer, pointer, length).set(input);
      },
      write(pointer: number, length: number) {
        result = JSON.parse(decode(pointer, length)) as CssDiagnostic[];
      },
    },
  };
  // wasm-bindgen's imports; an unoptimized checkout build still lists the unused ones.
  for (const { module: from, name } of WebAssembly.Module.imports(module)) {
    if (from === "sk") continue;
    (imports[from] ??= {})[name] = /^__wbg___wbindgen_throw_/.test(name)
      ? (pointer: number, length: number) => {
          throw new Error(decode(pointer, length));
        }
      : () => {
          throw new Error(`The CSS checker called ${from}.${name}, which it does not provide.`);
        };
  }
  const exports = new WebAssembly.Instance(module, imports).exports as unknown as Checker;
  return (block: { nodes: CssNode[]; tag: string | null; tailwind: boolean }) => {
    input = new TextEncoder().encode(JSON.stringify(block));
    exports.check(input.length);
    return result;
  };
}

let engine: ReturnType<typeof loadEngine> | undefined;
export function inspectCss(block: Parameters<ReturnType<typeof loadEngine>>[0]): CssDiagnostic[] {
  return (engine ??= loadEngine())(block);
}
