// What of a source file runs in the production bundle. The bundler keeps code behind a guard it could have folded
// (`if (DEV)` with esm-env's `DEV` resolved to `false` in another module), so the bundle itself is read: a finding is
// dropped when the code it maps to sits under a guard that is constant there, or in a function all of whose
// references in the bundle do. The bundle holds compiled components too, so no caller is out of sight.

import { resolve } from "node:path";
import type ts from "typescript";
import type { Range } from "./schema";
import type { ModuleGraph } from "./graph";
import type { TypeScript } from "./program";

type Bundle = NonNullable<ModuleGraph["bundle"]>;
/** [original line, original column, generated line, generated column], zero-based. */
type Segment = [number, number, number, number];

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const MAX_DEPTH = 40;

function decode(bundle: Bundle, key: (path: string) => string): Map<string, Segment[]> {
  const bySource = new Map<number, Segment[]>();
  let source = 0;
  let line = 0;
  let column = 0;
  const groups = bundle.mappings.split(";");
  for (let genLine = 0; genLine < groups.length; genLine++) {
    let genColumn = 0;
    for (const segment of groups[genLine]!.split(",")) {
      if (!segment) continue;
      const fields: number[] = [];
      for (let i = 0, shift = 0, value = 0; i < segment.length; i++) {
        const digit = B64.indexOf(segment[i]!);
        value += (digit & 31) << shift;
        if (digit & 32) {
          shift += 5;
          continue;
        }
        fields.push(value & 1 ? -(value >>> 1) : value >>> 1);
        shift = 0;
        value = 0;
      }
      genColumn += fields[0] ?? 0;
      if (fields.length < 4) continue;
      source += fields[1]!;
      line += fields[2]!;
      column += fields[3]!;
      const list = bySource.get(source) ?? [];
      list.push([line, column, genLine, genColumn]);
      bySource.set(source, list);
    }
  }
  const out = new Map<string, Segment[]>();
  // A minifier can remove an entire module but leave its original source in the map.
  if (bySource.size) {
    for (const path of bundle.sources) if (path) out.set(key(path), []);
  }
  for (const [index, list] of bySource) {
    const path = bundle.sources[index];
    if (!path) continue;
    out.set(
      key(path),
      list.sort((a, b) => a[0] - b[0] || a[1] - b[1]),
    );
  }
  return out;
}

export class BundleLiveness {
  private segments: Map<string, Segment[]> | null = null;
  private file: ts.SourceFile | null = null;
  private checker: ts.TypeChecker | null = null;
  private byName: Map<string, ts.Identifier[]> | null = null;
  private readonly functions = new Map<ts.Node, boolean>();

  constructor(
    private readonly t: TypeScript,
    private readonly bundle: Bundle,
  ) {}

  private key(path: string): string {
    return this.t.sys.useCaseSensitiveFileNames ? resolve(path) : resolve(path).toLowerCase();
  }

  /** False when the code at `range` of `path` is in the bundle but cannot run there. */
  live(path: string, range: Range): boolean {
    this.segments ??= decode(this.bundle, (p) => this.key(p));
    const segments = this.segments.get(this.key(path));
    if (!segments) return true;
    if (!segments.length) return false;
    const at = this.generated(segments, range);
    if (!at) return true;
    this.parse();
    const offset = this.file!.getPositionOfLineAndCharacter(at[0], at[1]);
    return !this.dead(this.nodeAt(offset), 0);
  }

  private generated(segments: Segment[], range: Range): [number, number] | null {
    const before = (s: Segment, line: number, character: number) => s[0] < line || (s[0] === line && s[1] < character);
    let lo = 0;
    let hi = segments.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (before(segments[mid]!, range.start.line, range.start.character)) lo = mid + 1;
      else hi = mid;
    }
    const inside = segments[lo];
    if (inside && before(inside, range.end.line, range.end.character + 1)) return [inside[2], inside[3]];
    // A position the bundler mapped only at the start of its line.
    const previous = segments[lo - 1];
    return previous && previous[0] === range.start.line ? [previous[2], previous[3]] : null;
  }

  private parse(): void {
    if (this.file) return;
    const t = this.t;
    const name = "/sk-check-bundle.js";
    const file = t.createSourceFile(name, this.bundle.code, t.ScriptTarget.Latest, true, t.ScriptKind.JS);
    const options: ts.CompilerOptions = { allowJs: true, noLib: true, noResolve: true, types: [] };
    const host = t.createCompilerHost(options);
    host.getSourceFile = (path) => (path === name ? file : undefined);
    host.fileExists = (path) => path === name;
    host.readFile = (path) => (path === name ? this.bundle.code : undefined);
    this.checker = t.createProgram({ rootNames: [name], options, host }).getTypeChecker();
    this.file = file;
  }

  private nodeAt(offset: number): ts.Node {
    let node: ts.Node = this.file!;
    for (let found = true; found;) {
      found = false;
      this.t.forEachChild(node, (child) => {
        if (found || offset < child.getStart(this.file!) || offset >= child.end) return;
        node = child;
        found = true;
      });
    }
    return node;
  }

  private dead(node: ts.Node, depth: number): boolean {
    const t = this.t;
    for (let cur = node; cur.parent; cur = cur.parent) {
      const p = cur.parent;
      if (t.isIfStatement(p) && cur !== p.expression) {
        const test = this.truth(p.expression, depth);
        if (test === (cur === p.elseStatement)) return true;
      } else if (t.isConditionalExpression(p) && cur !== p.condition) {
        const test = this.truth(p.condition, depth);
        if (test === (cur === p.whenFalse)) return true;
      } else if (t.isBinaryExpression(p) && cur === p.right) {
        const op = p.operatorToken.kind;
        const left = this.truth(p.left, depth);
        if (op === t.SyntaxKind.AmpersandAmpersandToken && left === false) return true;
        if (op === t.SyntaxKind.BarBarToken && left === true) return true;
      }
      if (t.isFunctionDeclaration(cur) || t.isFunctionExpression(cur) || t.isArrowFunction(cur)) {
        const verdict = this.uncalled(cur, depth);
        if (verdict !== undefined) return verdict;
      }
    }
    return false;
  }

  /** For a named function: whether every reference to it is dead; undefined when it has no name to follow. */
  private uncalled(fn: ts.FunctionLikeDeclaration, depth: number): boolean | undefined {
    const t = this.t;
    const holder = t.isFunctionDeclaration(fn)
      ? fn.name
      : t.isVariableDeclaration(fn.parent) && fn.parent.initializer === fn && t.isIdentifier(fn.parent.name)
        ? fn.parent.name
        : undefined;
    if (!holder) return undefined;
    const known = this.functions.get(fn);
    if (known !== undefined) return known;
    if (depth > MAX_DEPTH) return false;
    // While its callers are judged, a recursive call counts as live.
    this.functions.set(fn, false);
    const refs = this.references(holder);
    const verdict = refs.length > 0 && refs.every((ref) => this.dead(ref, depth + 1));
    this.functions.set(fn, verdict);
    return verdict;
  }

  private references(name: ts.Identifier): ts.Identifier[] {
    const t = this.t;
    const checker = this.checker!;
    if (!this.byName) {
      this.byName = new Map();
      const visit = (node: ts.Node): void => {
        if (t.isIdentifier(node)) {
          const list = this.byName!.get(node.text) ?? [];
          list.push(node);
          this.byName!.set(node.text, list);
        }
        t.forEachChild(node, visit);
      };
      visit(this.file!);
    }
    const symbolOf = (id: ts.Identifier) =>
      t.isShorthandPropertyAssignment(id.parent)
        ? checker.getShorthandAssignmentValueSymbol(id.parent)
        : checker.getSymbolAtLocation(id);
    const symbol = symbolOf(name);
    if (!symbol) return [];
    return (this.byName.get(name.text) ?? []).filter((id) => id !== name && symbolOf(id) === symbol);
  }

  /** The truthiness of `expr` wherever it runs in the bundle, or undefined when it is not constant. */
  private truth(expr: ts.Expression, depth: number): boolean | undefined {
    const value = this.constant(expr, depth);
    return value === NOT_CONSTANT ? undefined : !!value;
  }

  private constant(expr: ts.Expression, depth: number): unknown {
    const t = this.t;
    if (depth > MAX_DEPTH) return NOT_CONSTANT;
    if (t.isParenthesizedExpression(expr)) return this.constant(expr.expression, depth);
    if (expr.kind === t.SyntaxKind.TrueKeyword) return true;
    if (expr.kind === t.SyntaxKind.FalseKeyword) return false;
    if (expr.kind === t.SyntaxKind.NullKeyword) return null;
    if (t.isNumericLiteral(expr)) return Number(expr.text);
    if (t.isStringLiteral(expr) || t.isNoSubstitutionTemplateLiteral(expr)) return expr.text;
    if (t.isVoidExpression(expr)) return undefined;
    if (t.isPrefixUnaryExpression(expr) && expr.operator === t.SyntaxKind.ExclamationToken) {
      const value = this.constant(expr.operand, depth + 1);
      return value === NOT_CONSTANT ? NOT_CONSTANT : !value;
    }
    if (t.isBinaryExpression(expr)) {
      const op = expr.operatorToken.kind;
      const left = this.constant(expr.left, depth + 1);
      if (left === NOT_CONSTANT) return NOT_CONSTANT;
      if (op === t.SyntaxKind.AmpersandAmpersandToken) return left ? this.constant(expr.right, depth + 1) : left;
      if (op === t.SyntaxKind.BarBarToken) return left ? left : this.constant(expr.right, depth + 1);
      const right = this.constant(expr.right, depth + 1);
      if (right === NOT_CONSTANT) return NOT_CONSTANT;
      if (op === t.SyntaxKind.EqualsEqualsEqualsToken) return left === right;
      if (op === t.SyntaxKind.ExclamationEqualsEqualsToken) return left !== right;
      if (op === t.SyntaxKind.EqualsEqualsToken) return left == right;
      if (op === t.SyntaxKind.ExclamationEqualsToken) return left != right;
      return NOT_CONSTANT;
    }
    if (t.isIdentifier(expr)) return this.binding(expr, depth);
    return NOT_CONSTANT;
  }

  /** A variable initialised to a constant and never written again. */
  private binding(id: ts.Identifier, depth: number): unknown {
    const t = this.t;
    const symbol = this.checker!.getSymbolAtLocation(id);
    const declarations = symbol?.declarations ?? [];
    const decl = declarations[0];
    if (declarations.length !== 1 || !decl || !t.isVariableDeclaration(decl) || !decl.initializer) {
      return NOT_CONSTANT;
    }
    if (!t.isIdentifier(decl.name)) return NOT_CONSTANT;
    for (const ref of this.references(decl.name)) {
      const p = ref.parent;
      const written =
        (t.isBinaryExpression(p) &&
          p.left === ref &&
          p.operatorToken.kind >= t.SyntaxKind.FirstAssignment &&
          p.operatorToken.kind <= t.SyntaxKind.LastAssignment) ||
        ((t.isPrefixUnaryExpression(p) || t.isPostfixUnaryExpression(p)) &&
          (p.operator === t.SyntaxKind.PlusPlusToken || p.operator === t.SyntaxKind.MinusMinusToken));
      if (written) return NOT_CONSTANT;
    }
    return this.constant(decl.initializer, depth + 1);
  }
}

const NOT_CONSTANT = Symbol("not constant");
