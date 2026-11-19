import { readFileSync } from "node:fs";
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

function loadEngine() {
  const module = new WebAssembly.Module(
    brotliDecompressSync(readFileSync(new URL("./sk-css.wasm.br", import.meta.url))),
  );
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
  const binding = WebAssembly.Module.imports(module).find((i) => i.module === "__wbindgen_placeholder__");
  if (binding)
    imports[binding.module] = {
      [binding.name]: (pointer: number, length: number) => {
        throw new Error(decode(pointer, length));
      },
    };
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
