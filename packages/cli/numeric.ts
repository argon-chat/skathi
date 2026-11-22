// `skathi:numeric` calls inlined by `sk build`: `i32.add(a, b)` becomes `((a) + (b)) | 0`, the body of
// the runtime function, so the result is the same for every input and the call costs nothing. Only
// the forms that use each argument once are inlined; the rest stay calls.

import { resolve } from "node:path";
import type ts from "typescript";
import { buildProgram, programConfig, type ProgramConfig, type TypeScript } from "./check/program";
import { stdModuleFile } from "./runtime-path";

export const NUMERIC_MODULE = "skathi:numeric";

/** The cheap test before any TypeScript: the module named in a string literal. */
export const importsNumeric = (source: string) => /["']skathi:numeric["']/.test(source);

export interface NumericKind {
  name: string;
  bits: 8 | 16 | 32 | 64;
  signed: boolean;
  float: boolean;
}

export const NUMERIC_KINDS: ReadonlyMap<string, NumericKind> = new Map(
  (
    [
      ["i8", 8, true, false],
      ["i16", 16, true, false],
      ["i32", 32, true, false],
      ["u8", 8, false, false],
      ["u16", 16, false, false],
      ["u32", 32, false, false],
      ["i64", 64, true, false],
      ["u64", 64, false, false],
      ["f16", 16, true, true],
      ["f32", 32, true, true],
    ] as const
  ).map(([name, bits, signed, float]) => [name, { name, bits, signed, float }]),
);

const sameFile = (a: string, b: string) => resolve(a).toLowerCase() === resolve(b).toLowerCase();

/** The export of `skathi:numeric` an expression names (`i32`, `n.u8`), or null. */
export function numericExport(
  t: TypeScript,
  checker: ts.TypeChecker,
  node: ts.Expression,
  numericFile: string,
): NumericKind | null {
  if (!t.isIdentifier(node) && !(t.isPropertyAccessExpression(node) && !node.questionDotToken)) return null;
  let symbol = checker.getSymbolAtLocation(node);
  if (symbol && symbol.flags & t.SymbolFlags.Alias) symbol = checker.getAliasedSymbol(symbol);
  const declaration = symbol?.valueDeclaration;
  if (!declaration || !t.isVariableDeclaration(declaration)) return null;
  if (!sameFile(declaration.getSourceFile().fileName, numericFile)) return null;
  return NUMERIC_KINDS.get(symbol!.name) ?? null;
}

/** The kind a value's type is branded with by `skathi:numeric` (`i32`, `u8`), or null for any other type. */
export function brandOf(checker: ts.TypeChecker, type: ts.Type, numericFile: string): string | null {
  if (type.isUnion()) return null;
  const brand = type.getProperty("__kind");
  const declaration = brand?.valueDeclaration;
  if (!brand || !declaration || !sameFile(declaration.getSourceFile().fileName, numericFile)) return null;
  const kind = checker.getTypeOfSymbol(brand);
  return kind.isStringLiteral() && NUMERIC_KINDS.has(kind.value) ? kind.value : null;
}

type Emit = (args: string[]) => string;

function wrap(kind: NumericKind, e: string): string {
  if (kind.float) return `${kind.bits === 32 ? "Math.fround" : "Math.f16round"}(${e})`;
  if (kind.bits === 64) return `BigInt.${kind.signed ? "asIntN" : "asUintN"}(64, ${e})`;
  if (kind.bits === 32) return kind.signed ? `(${e}) | 0` : `(${e}) >>> 0`;
  const spare = 32 - kind.bits;
  return kind.signed ? `(${e}) << ${spare} >> ${spare}` : `(${e}) & ${(1 << kind.bits) - 1}`;
}

/** The inlined form of `<kind>.<method>(…)`, by argument count; undefined when the method stays a call. */
function methodForm(kind: NumericKind, method: string): [number, Emit] | undefined {
  const w = (e: string) => wrap(kind, e);
  const binary = (op: string): [number, Emit] => [2, ([a, b]) => w(`${a} ${op} ${b}`)];
  if (kind.float) {
    if (method === "add") return binary("+");
    if (method === "sub") return binary("-");
    if (method === "mul") return binary("*");
    if (method === "div") return binary("/");
    if (method === "fma") return [3, ([a, b, c]) => w(`${a} * ${b} + ${c}`)];
    return undefined;
  }
  switch (method) {
    case "add":
      return binary("+");
    case "sub":
      return binary("-");
    case "mul":
      return kind.bits === 64 ? binary("*") : [2, ([a, b]) => w(`Math.imul(${a}, ${b})`)];
    case "and":
      return binary("&");
    case "or":
      return binary("|");
    case "xor":
      return binary("^");
    case "neg":
      return [1, ([a]) => w(`-${a}`)];
    case "not":
      return [1, ([a]) => w(`~${a}`)];
    case "lt":
      return [2, ([a, b]) => `${a} < ${b}`];
  }
  if (kind.bits === 64) return undefined;
  const count = (n: string | undefined) => `(${n} & ${kind.bits - 1})`;
  switch (method) {
    case "shl":
      return [2, ([a, n]) => w(`${a} << ${count(n)}`)];
    case "shr":
      return [2, ([a, n]) => w(`${a} ${kind.signed ? ">>" : ">>>"} ${count(n)}`)];
    case "clz": {
      const mask = kind.bits === 32 ? 0xffffffff : (1 << kind.bits) - 1;
      return [1, ([a]) => `Math.clz32((${a} & ${mask}) >>> 0) - ${32 - kind.bits}`];
    }
  }
  return undefined;
}

function constant(kind: NumericKind, name: string): string | undefined {
  if (name === "BITS" && !kind.float) return String(kind.bits);
  if (name !== "MIN" && name !== "MAX") return undefined;
  if (kind.float) return undefined;
  const big = BigInt(kind.bits);
  const value =
    name === "MIN" ? (kind.signed ? -(2n ** (big - 1n)) : 0n) : kind.signed ? 2n ** (big - 1n) - 1n : 2n ** big - 1n;
  return kind.bits === 64 ? `${value}n` : String(value);
}

const NEEDS_MATH = (kind: NumericKind, method: string) => kind.float || method === "mul" || method === "clz";

/**
 * Names the file declares anywhere (or uses as a member name, which only costs an inlining), so an
 * inlined `Math.*` or `BigInt.*` never meets a local of that name.
 */
function declaredNames(t: TypeScript, file: ts.SourceFile): Set<string> {
  const names = new Set<string>();
  const visit = (node: ts.Node): void => {
    if (
      t.isIdentifier(node) &&
      (node.parent as { name?: ts.Node }).name === node &&
      !t.isPropertyAccessExpression(node.parent)
    )
      names.add(node.text);
    t.forEachChild(node, visit);
  };
  visit(file);
  return names;
}

interface Plan {
  args: readonly ts.Expression[];
  emit: Emit;
}

/** The file's text with every `skathi:numeric` call that can be inlined replaced; null when there is none. */
export function lowerNumeric(
  t: TypeScript,
  checker: ts.TypeChecker,
  file: ts.SourceFile,
  numericFile: string = stdModuleFile(NUMERIC_MODULE),
): { text: string; count: number } | null {
  const text = file.text;
  const local = declaredNames(t, file);
  let count = 0;

  const bigint = (node: ts.Expression) => {
    const type = checker.getTypeAtLocation(node);
    return !(type.flags & t.TypeFlags.Any) && checker.isTypeAssignableTo(type, checker.getBigIntType());
  };

  const plan = (node: ts.Node): Plan | null => {
    if (t.isPropertyAccessExpression(node) && !node.questionDotToken) {
      const parent = node.parent;
      const written =
        (t.isBinaryExpression(parent) &&
          parent.left === node &&
          parent.operatorToken.kind >= t.SyntaxKind.FirstAssignment &&
          parent.operatorToken.kind <= t.SyntaxKind.LastAssignment) ||
        t.isDeleteExpression(parent) ||
        t.isPrefixUnaryExpression(parent) ||
        t.isPostfixUnaryExpression(parent);
      if (written) return null;
      const kind = numericExport(t, checker, node.expression, numericFile);
      const value = kind && constant(kind, node.name.text);
      return value === undefined || value === null ? null : { args: [], emit: () => value };
    }
    if (!t.isCallExpression(node) || node.questionDotToken || node.typeArguments) return null;
    if (node.arguments.some((a) => t.isSpreadElement(a))) return null;
    const callee = node.expression;
    const direct = numericExport(t, checker, callee, numericFile);
    if (direct) {
      if (node.arguments.length !== 1) return null;
      if (direct.bits === 64 && !bigint(node.arguments[0]!)) return null;
      if (direct.float && local.has("Math")) return null;
      if (direct.bits === 64 && local.has("BigInt")) return null;
      return { args: node.arguments, emit: ([x]) => wrap(direct, x!) };
    }
    if (!t.isPropertyAccessExpression(callee) || callee.questionDotToken) return null;
    const kind = numericExport(t, checker, callee.expression, numericFile);
    if (!kind) return null;
    const form = methodForm(kind, callee.name.text);
    if (!form || form[0] !== node.arguments.length) return null;
    if (NEEDS_MATH(kind, callee.name.text) && kind.bits !== 64 && local.has("Math")) return null;
    if (kind.bits === 64 && callee.name.text !== "lt" && local.has("BigInt")) return null;
    return { args: node.arguments, emit: form[1] };
  };

  const newlines = (from: number, to: number) => text.slice(from, to).replace(/[^\n]/g, "");

  const emit = (node: ts.Node, p: Plan): string => {
    count++;
    let at = node.getStart(file);
    const args = p.args.map((arg) => {
      const start = arg.getStart(file);
      const lead = newlines(at, start);
      at = arg.end;
      return `(${lead}${render(arg)})`;
    });
    // Line breaks the call spanned stay inside the replacement, so later lines keep their numbers.
    return `(${p.emit(args)}${newlines(at, node.end)})`;
  };

  const render = (node: ts.Node): string => {
    const own = plan(node);
    if (own) return emit(node, own);
    let out = "";
    let pos = node.getStart(file);
    const visit = (child: ts.Node): void => {
      const p = plan(child);
      if (!p) return void t.forEachChild(child, visit);
      out += text.slice(pos, child.getStart(file)) + emit(child, p);
      pos = child.end;
    };
    t.forEachChild(node, visit);
    return out + text.slice(pos, node.end);
  };

  let out = "";
  let pos = 0;
  const top = (child: ts.Node): void => {
    const p = plan(child);
    if (!p) return void t.forEachChild(child, top);
    out += text.slice(pos, child.getStart(file)) + emit(child, p);
    pos = child.end;
  };
  t.forEachChild(file, top);
  if (!count) return null;
  return { text: out + text.slice(pos), count };
}

export interface NumericLoweringOptions {
  typescript: TypeScript;
  appRoot: string;
  entry: string;
  aliases: Record<string, string>;
}

/**
 * `sk build`'s pass. Each file that imports `skathi:numeric` is the root of a program with the
 * app's compiler options; the files parsed for one are reused by the next.
 */
export class NumericLowering {
  private program: ts.Program | undefined;
  private config: ProgramConfig | undefined;
  private readonly parsed = new Map<string, ts.SourceFile>();
  inlined = 0;

  constructor(private readonly options: NumericLoweringOptions) {}

  lower(path: string, source: string): string | null {
    if (!importsNumeric(source)) return null;
    const { typescript: t, appRoot, entry } = this.options;
    if (!this.config) {
      const app = programConfig(t, appRoot, entry);
      // What is inlined depends on symbols and on `bigint`, which need no lib or ambient types.
      this.config = { ...app, options: { ...app.options, noLib: true, types: [] }, roots: [] };
    }
    const built = buildProgram({
      typescript: t,
      appRoot,
      entry,
      units: [
        {
          path: resolve(path),
          programPath: resolve(path),
          text: source,
          typed: /\.[cm]?tsx?$/i.test(path),
          owner: null,
        },
      ],
      aliases: this.options.aliases,
      declarations: [],
      config: this.config,
      oldProgram: this.program,
      sourceFiles: this.parsed,
    });
    this.program = built.program;
    const file = built.program.getSourceFile(resolve(path));
    if (!file) return null;
    const lowered = lowerNumeric(t, built.checker, file);
    if (!lowered) return null;
    this.inlined += lowered.count;
    return lowered.text;
  }
}
