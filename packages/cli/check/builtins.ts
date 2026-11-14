import { readFileSync } from "node:fs";
import { join } from "node:path";
import type ts from "typescript";
import type { TypeScript } from "./program";

interface Table {
  table: string;
  engine: string;
  locked: string[];
  replaceable: string[];
  lockedObjects: string[];
  objects: string[];
}
const data = JSON.parse(readFileSync(join(import.meta.dir, "builtins.json"), "utf8")) as Table;

/** Engine built-ins whose replacement or deletion throws under AOT (`Array.prototype.push`, `Map`). */
export const LOCKED: ReadonlySet<string> = new Set(data.locked);
/** Engine built-ins that stay replaceable (`Date.now`, `Promise`). */
export const REPLACEABLE: ReadonlySet<string> = new Set(data.replaceable);
/** Objects that take no new members (`Object.prototype`). */
export const LOCKED_OBJECTS: ReadonlySet<string> = new Set(data.lockedObjects);
/** Prototypes, constructors and namespaces. */
export const OBJECTS: ReadonlySet<string> = new Set(data.objects);
export const TABLE_ID: string = data.table;

const GLOBAL_OBJECTS = new Set(["globalThis", "window", "self", "global"]);
const WELL_KNOWN_SYMBOL = /^Symbol\.[A-Za-z]+$/;

/** `holder.member` in the table's notation. */
export const memberPath = (holder: string, member: string) =>
  holder === "globalThis" ? member : WELL_KNOWN_SYMBOL.test(member) ? `${holder}[${member}]` : `${holder}.${member}`;

/** Resolves expressions to the engine built-in they name, using the checker to see through shadowing. */
export class Builtins {
  constructor(
    private readonly typescript: TypeScript,
    private readonly program: ts.Program,
    private readonly checker: ts.TypeChecker,
  ) {}

  private globalDeclaration(declaration: ts.Declaration): boolean {
    const file = declaration.getSourceFile();
    if (this.program.isSourceFileDefaultLibrary(file)) return true;
    if (!file.isDeclarationFile) return false;
    if (!this.typescript.isExternalModule(file)) return true;
    for (let node: ts.Node | undefined = declaration; node; node = node.parent) {
      if (this.typescript.isModuleDeclaration(node) && node.name.text === "global") return true;
    }
    return false;
  }

  /** The identifier refers to the global binding of its name, not to a local or an import. */
  isGlobal(id: ts.Identifier): boolean {
    let symbol: ts.Symbol | undefined;
    try {
      symbol = this.checker.getSymbolAtLocation(id);
    } catch {
      symbol = undefined;
    }
    const declarations = symbol?.declarations ?? [];
    return declarations.every((d) => this.globalDeclaration(d));
  }

  /** A property name as written, if it is a constant: `.push`, `["push"]`, `[Symbol.iterator]`. */
  memberName(node: ts.Expression | ts.PropertyName | ts.MemberName): string | null {
    const t = this.typescript;
    if (t.isIdentifier(node) || t.isPrivateIdentifier(node)) return t.isIdentifier(node) ? node.text : null;
    if (t.isStringLiteral(node) || t.isNoSubstitutionTemplateLiteral(node) || t.isNumericLiteral(node))
      return node.text;
    if (t.isComputedPropertyName(node)) return this.memberName(node.expression);
    if (t.isPropertyAccessExpression(node) && t.isIdentifier(node.expression) && node.expression.text === "Symbol") {
      return this.isGlobal(node.expression) ? `Symbol.${node.name.text}` : null;
    }
    return null;
  }

  /** "globalThis", a global's name, or a dotted built-in path; null when the expression is not a built-in. */
  path(node: ts.Expression, depth = 0): string | null {
    const t = this.typescript;
    if (depth > 8) return null;
    if (t.isParenthesizedExpression(node) || t.isNonNullExpression(node) || t.isAsExpression(node))
      return this.path(node.expression, depth + 1);
    if (t.isIdentifier(node)) {
      if (this.isGlobal(node)) return GLOBAL_OBJECTS.has(node.text) ? "globalThis" : node.text;
      return this.alias(node, depth);
    }
    if (t.isPropertyAccessExpression(node) || t.isElementAccessExpression(node)) {
      const name = this.memberName(t.isPropertyAccessExpression(node) ? node.name : node.argumentExpression);
      if (name === null) return null;
      const holder = this.path(node.expression, depth + 1);
      if (holder === null) return null;
      if (holder === "globalThis" && GLOBAL_OBJECTS.has(name)) return "globalThis";
      return memberPath(holder, name);
    }
    return null;
  }

  /** `const AP = Array.prototype`: the built-in a constant names. */
  private alias(id: ts.Identifier, depth: number): string | null {
    const t = this.typescript;
    const declaration = this.checker.getSymbolAtLocation(id)?.valueDeclaration;
    if (
      !declaration ||
      !t.isVariableDeclaration(declaration) ||
      !declaration.initializer ||
      !t.isIdentifier(declaration.name) ||
      !(t.getCombinedNodeFlags(declaration) & t.NodeFlags.Const)
    )
      return null;
    return this.path(declaration.initializer, depth + 1);
  }
}
