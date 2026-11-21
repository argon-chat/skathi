import type ts from "typescript";
import { rangeOf, type Finding } from "./finding";
import type { TypeScript, Unit } from "./program";

/** `"permissions"` of sk.config.json as the check needs it: names on any platform. */
export interface Allowed {
  ffi: ReadonlySet<string>;
  ffiAddress: boolean;
}

const MODULE = "skathi:ffi";

export function allowedOf(raw: unknown): Allowed {
  const p = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const names = new Set<string>();
  for (const entry of Array.isArray(p.ffi) ? (p.ffi as unknown[]) : []) {
    const name = entry && typeof entry === "object" ? (entry as Record<string, unknown>).name : undefined;
    if (typeof name === "string") names.add(name);
  }
  return { ffi: names, ffiAddress: p.ffiAddress === true };
}

export class PermissionRules {
  constructor(
    private readonly typescript: TypeScript,
    private readonly checker: ts.TypeChecker,
    private readonly allowed: Allowed,
  ) {}

  run(unit: Unit, file: ts.SourceFile, out: Finding[]): void {
    const t = this.typescript;
    const report = (code: string, message: string, node: ts.Node, help?: string) =>
      out.push({
        code,
        message,
        file: unit.path,
        range: rangeOf(file, node.getStart(file), node.getEnd()),
        help,
        related: [],
      });

    const imports: ts.Expression[] = [];
    const bindings = new Set<ts.Node>();
    const namespaces = new Set<ts.Node>();
    for (const statement of file.statements) {
      if (!t.isImportDeclaration(statement) || !t.isStringLiteral(statement.moduleSpecifier)) continue;
      if (statement.moduleSpecifier.text !== MODULE || statement.importClause?.isTypeOnly) continue;
      imports.push(statement.moduleSpecifier);
      const named = statement.importClause?.namedBindings;
      if (named && t.isNamespaceImport(named)) namespaces.add(named);
      else if (named)
        for (const element of named.elements)
          if (!element.isTypeOnly && (element.propertyName ?? element.name).text === "dlopen") bindings.add(element);
    }
    const calls: ts.CallExpression[] = [];
    const visit = (node: ts.Node): void => {
      if (t.isCallExpression(node)) {
        const spec = node.arguments[0];
        if (node.expression.kind === t.SyntaxKind.ImportKeyword && spec && t.isStringLiteralLike(spec)) {
          if (spec.text === MODULE) imports.push(spec);
        } else if (this.isDlopen(node.expression, bindings, namespaces)) calls.push(node);
      }
      t.forEachChild(node, visit);
    };
    visit(file);
    const first = imports[0];
    if (!first) return;

    if (unit.owner)
      report(
        "SK6005",
        `the package \`${unit.owner.name}\` imports \`${MODULE}\`: it calls native libraries with this app's permissions`,
        first,
      );
    if (!this.allowed.ffi.size && !this.allowed.ffiAddress) {
      report(
        "SK6001",
        `\`${MODULE}\` is imported, but sk.config.json has no "permissions.ffi": every dlopen() throws`,
        first,
      );
      return;
    }
    for (const call of calls) {
      const name = call.arguments[0];
      if (!name) continue;
      const values = this.literals(name);
      if (!values) {
        report("SK6003", "the library name given to `dlopen()` is not a literal, so it cannot be checked", name);
        continue;
      }
      for (const value of values) {
        if (this.allowed.ffi.has(value)) continue;
        const known = [...this.allowed.ffi].map((n) => JSON.stringify(n)).join(", ");
        report(
          "SK6002",
          `\`dlopen(${JSON.stringify(value)})\`: sk.config.json "permissions.ffi" has no entry named ${JSON.stringify(value)}, so the call throws NotAllowedError`,
          name,
          `Add { "name": ${JSON.stringify(value)}, "path": "$APP/…" or "$SYSTEM/…" } to "permissions.ffi"${known ? `, or use a listed name: ${known}` : ""}.`,
        );
      }
    }
  }

  private isDlopen(callee: ts.Expression, bindings: Set<ts.Node>, namespaces: Set<ts.Node>): boolean {
    const t = this.typescript;
    const declared = (node: ts.Node, among: Set<ts.Node>) =>
      among.size > 0 && (this.checker.getSymbolAtLocation(node)?.declarations ?? []).some((d) => among.has(d));
    if (t.isIdentifier(callee)) return declared(callee, bindings);
    return (
      t.isPropertyAccessExpression(callee) && callee.name.text === "dlopen" && declared(callee.expression, namespaces)
    );
  }

  /** The strings the expression can be, when the types say exactly which. */
  private literals(node: ts.Expression): string[] | null {
    const t = this.typescript;
    if (t.isStringLiteralLike(node)) return [node.text];
    let type: ts.Type;
    try {
      type = this.checker.getTypeAtLocation(node);
    } catch {
      return null;
    }
    const parts = type.isUnion() ? type.types : [type];
    const values: string[] = [];
    for (const part of parts) {
      if (!part.isStringLiteral()) return null;
      values.push(part.value);
    }
    return values.length ? values : null;
  }
}
