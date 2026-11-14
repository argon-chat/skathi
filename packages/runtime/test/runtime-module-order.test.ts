// src/runtime/ modules import each other in cycles, and a bundle evaluates a cycle in whatever order its first import
// edge picks, with `const`s hoisted to `var`: a too-early read is `undefined`, not a TDZ error, and a hook installed
// before its owner's `let x = null` runs is silently cleared. So code that runs while a module evaluates (top level,
// static fields, and the functions those call) may read another module's bindings only when that module cannot reach
// back to it; shared mutable state lives in leaves such as runtime/state.ts.
import { expect, test } from "bun:test";
import { readdirSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import ts from "typescript";

const PKG = join(import.meta.dir, "..");
const ENTRY = join(PKG, "src", "runtime.ts");

const norm = (p: string) => resolve(p).replace(/\\/g, "/").toLowerCase();
const parsed = ts.parseJsonConfigFileContent(
  ts.readConfigFile(join(PKG, "tsconfig.json"), ts.sys.readFile).config,
  ts.sys,
  PKG,
);
const program = ts.createProgram(parsed.fileNames, parsed.options);
const checker = program.getTypeChecker();
const files = new Map(
  program
    .getSourceFiles()
    .filter((f) => !f.isDeclarationFile)
    .map((f) => [norm(f.fileName), f]),
);
const short = (f: ts.SourceFile) => relative(join(PKG, "src"), f.fileName).replace(/\\/g, "/");
const aliased = (sym: ts.Symbol | undefined) =>
  sym && sym.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(sym) : sym;

const depsOf = new Map<ts.SourceFile, ts.SourceFile[]>();
function valueDeps(f: ts.SourceFile): ts.SourceFile[] {
  let out = depsOf.get(f);
  if (out) return out;
  out = [];
  for (const s of f.statements) {
    if (!ts.isImportDeclaration(s) && !(ts.isExportDeclaration(s) && s.moduleSpecifier)) continue;
    const spec = (s.moduleSpecifier as ts.StringLiteral).text;
    const dep = spec.startsWith(".") ? files.get(norm(resolve(dirname(f.fileName), spec) + ".ts")) : undefined;
    if (!dep) continue;
    if (ts.isImportDeclaration(s)) {
      const c = s.importClause;
      if (c?.isTypeOnly) continue;
      const nb = c?.namedBindings;
      if (c && !c.name && nb && ts.isNamedImports(nb)) {
        const values = nb.elements.filter(
          (e) => !e.isTypeOnly && aliased(checker.getSymbolAtLocation(e.name))!.flags & ts.SymbolFlags.Value,
        );
        if (!values.length) continue;
      }
    } else if (s.isTypeOnly) continue;
    out.push(dep);
  }
  depsOf.set(f, out);
  return out;
}

// ESM evaluation order: depth-first over value imports, a module already entered skipped.
const order: ts.SourceFile[] = [];
const entered = new Set<ts.SourceFile>();
(function visit(f: ts.SourceFile) {
  entered.add(f);
  for (const dep of valueDeps(f)) if (!entered.has(dep)) visit(dep);
  order.push(f);
})(files.get(norm(ENTRY))!);
const rank = new Map(order.map((f, i) => [f, i]));

const reachOf = new Map<ts.SourceFile, Set<ts.SourceFile>>();
function reaches(from: ts.SourceFile, to: ts.SourceFile): boolean {
  let r = reachOf.get(from);
  if (!r) {
    r = new Set();
    const stack = [...valueDeps(from)];
    while (stack.length) {
      const d = stack.pop()!;
      if (r.has(d)) continue;
      r.add(d);
      stack.push(...valueDeps(d));
    }
    reachOf.set(from, r);
  }
  return r.has(to);
}

interface Decl {
  file: ts.SourceFile;
  node: ts.Node;
  fn: boolean;
}
const decls = new Map<ts.Symbol, Decl>();
for (const f of order)
  for (const s of f.statements) {
    const add = (id: ts.Identifier, node: ts.Node, fn: boolean) => {
      const sym = checker.getSymbolAtLocation(id);
      if (sym) decls.set(sym, { file: f, node, fn });
    };
    if (ts.isFunctionDeclaration(s) && s.name) add(s.name, s, true);
    else if (ts.isClassDeclaration(s) && s.name) add(s.name, s, false);
    else if (ts.isVariableStatement(s))
      for (const d of s.declarationList.declarations) {
        const bind = (n: ts.BindingName) => {
          if (ts.isIdentifier(n)) add(n, d, false);
          else for (const e of n.elements) if (!ts.isOmittedExpression(e)) bind(e.name);
        };
        bind(d.name);
      }
  }

function inType(n: ts.Node): boolean {
  for (let p: ts.Node = n; p; p = p.parent) {
    if (ts.isTypeNode(p) || ts.isInterfaceDeclaration(p) || ts.isTypeAliasDeclaration(p)) return true;
    if (ts.isStatement(p) || ts.isSourceFile(p)) return false;
  }
  return false;
}

/** The binding an identifier reads, or undefined where it names a declaration, property or key instead. */
function readOf(id: ts.Identifier): ts.Symbol | undefined {
  const p = id.parent;
  if (ts.isShorthandPropertyAssignment(p) && p.name === id)
    return aliased(checker.getShorthandAssignmentValueSymbol(p));
  if (ts.isPropertyAccessExpression(p) && p.name === id) return undefined;
  if (ts.isBindingElement(p) && p.propertyName === id) return undefined;
  const named =
    ts.isVariableDeclaration(p) ||
    ts.isBindingElement(p) ||
    ts.isClassDeclaration(p) ||
    ts.isClassExpression(p) ||
    ts.isFunctionDeclaration(p) ||
    ts.isFunctionExpression(p) ||
    ts.isParameter(p) ||
    ts.isPropertyAssignment(p) ||
    ts.isMethodDeclaration(p) ||
    ts.isPropertyDeclaration(p) ||
    ts.isGetAccessor(p) ||
    ts.isSetAccessor(p);
  if (named && (p as ts.NamedDeclaration).name === id) return undefined;
  return aliased(checker.getSymbolAtLocation(id));
}

/** Identifiers evaluated when `n` runs; nested function bodies and instance fields run later. */
function eagerIds(n: ts.Node, out: ts.Identifier[], top = true): ts.Identifier[] {
  if (!top && ts.isFunctionLike(n) && !ts.isTypeNode(n)) return out;
  if (ts.isPropertyDeclaration(n) && !ts.getModifiers(n)?.some((m) => m.kind === ts.SyntaxKind.StaticKeyword))
    return out;
  if (ts.isIdentifier(n) && !inType(n)) out.push(n);
  n.forEachChild((c) => void eagerIds(c, out, false));
  return out;
}

const isCallee = (id: ts.Identifier) => {
  const p = id.parent;
  return (
    ((ts.isCallExpression(p) || ts.isNewExpression(p)) && p.expression === id) ||
    (ts.isTaggedTemplateExpression(p) && p.tag === id)
  );
};

/** Non-function bindings calling `fn` reads, through the functions it calls. */
const readsOf = new Map<ts.Node, Set<ts.Symbol>>();
function callReads(fn: ts.FunctionDeclaration): Set<ts.Symbol> {
  let out = readsOf.get(fn);
  if (out) return out;
  out = new Set();
  readsOf.set(fn, out);
  const ids: ts.Identifier[] = [];
  for (const p of fn.parameters) if (p.initializer) eagerIds(p.initializer, ids);
  if (fn.body) eagerIds(fn.body, ids);
  for (const id of ids) {
    const sym = readOf(id);
    const d = sym && decls.get(sym);
    if (!d) continue;
    if (!d.fn) out.add(sym!);
    else if (isCallee(id)) for (const x of callReads(d.node as ts.FunctionDeclaration)) out.add(x);
  }
  return out;
}

test("module-evaluation code reads only bindings that are certainly initialized", () => {
  const problems = new Set<string>();
  for (const f of order) {
    for (const s of f.statements) {
      if (
        ts.isFunctionDeclaration(s) ||
        ts.isInterfaceDeclaration(s) ||
        ts.isTypeAliasDeclaration(s) ||
        ts.isImportDeclaration(s) ||
        ts.isExportDeclaration(s)
      )
        continue;
      for (const id of eagerIds(s, [])) {
        const sym = readOf(id);
        const d = sym && decls.get(sym);
        if (!d) continue;
        const at = `${short(f)}:${f.getLineAndCharacterOfPosition(id.getStart(f)).line + 1}`;
        const check = (read: ts.Symbol, via: string) => {
          const src = decls.get(read)!;
          if (src.file === f) {
            if (src.node.getStart(f) > s.getStart(f))
              problems.add(`${at} reads ${read.name}${via} before its declaration`);
          } else if (rank.get(src.file)! > rank.get(f)!) {
            problems.add(`${at} reads ${read.name}${via} from ${short(src.file)}, which evaluates later`);
          } else if (f !== order.at(-1) && reaches(src.file, f)) {
            problems.add(`${at} reads ${read.name}${via} from ${short(src.file)}, which imports it back`);
          }
        };
        if (!d.fn) check(sym!, "");
        else if (isCallee(id))
          for (const x of callReads(d.node as ts.FunctionDeclaration)) check(x, ` (in ${sym!.name}())`);
      }
    }
  }
  expect([...problems]).toEqual([]);
});

test("every runtime module with values is evaluated", () => {
  const evaluated = new Set(order.map(short));
  const missing = readdirSync(join(PKG, "src", "runtime"))
    .map((name) => files.get(norm(join(PKG, "src", "runtime", name)))!)
    .filter((f) =>
      f.statements.some(
        (s) =>
          !ts.isInterfaceDeclaration(s) &&
          !ts.isTypeAliasDeclaration(s) &&
          !(ts.isImportDeclaration(s) && s.importClause?.isTypeOnly),
      ),
    )
    .map(short)
    .filter((name) => !evaluated.has(name));
  expect(missing).toEqual([]);
});
