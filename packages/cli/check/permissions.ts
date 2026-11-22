import type ts from "typescript";
import { rangeOf, type Finding } from "./finding";
import type { TypeScript, Unit } from "./program";

/** `"permissions"` of sk.config.json as the check needs it: names on any platform. */
export interface Allowed {
  ffi: ReadonlySet<string>;
  ffiAddress: boolean;
  process: ReadonlySet<string>;
  /** Entries that find their program on PATH or take any arguments. */
  processWide: readonly string[];
}

const FFI = "skathi:ffi";
const PROCESS = "skathi:process";

function names(list: unknown): Set<string> {
  const out = new Set<string>();
  for (const entry of Array.isArray(list) ? (list as unknown[]) : []) {
    const name = entry && typeof entry === "object" ? (entry as Record<string, unknown>).name : undefined;
    if (typeof name === "string") out.add(name);
  }
  return out;
}

function onPath(path: unknown): boolean {
  if (typeof path === "string") return path.startsWith("$PATH/");
  return !!path && typeof path === "object" && Object.values(path).some(onPath);
}

export function allowedOf(raw: unknown): Allowed {
  const p = raw && typeof raw === "object" ? (raw as Record<string, unknown>) : {};
  const processWide: string[] = [];
  for (const entry of Array.isArray(p.process) ? (p.process as unknown[]) : []) {
    if (!entry || typeof entry !== "object") continue;
    const e = entry as Record<string, unknown>;
    if (typeof e.name === "string" && (e.args === true || onPath(e.path))) processWide.push(e.name);
  }
  return { ffi: names(p.ffi), ffiAddress: p.ffiAddress === true, process: names(p.process), processWide };
}

interface Module {
  name: typeof FFI | typeof PROCESS;
  imports: ts.Expression[];
  /** Local names of the export whose calls take the checked name. */
  bindings: Set<ts.Node>;
  namespaces: Set<ts.Node>;
  calls: ts.CallExpression[];
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

    const modules = new Map<string, Module>(
      [FFI, PROCESS].map((name) => [
        name,
        { name, imports: [], bindings: new Set(), namespaces: new Set(), calls: [] } as Module,
      ]),
    );
    const exported = (m: Module) => (m.name === FFI ? "dlopen" : "Process");
    for (const statement of file.statements) {
      if (!t.isImportDeclaration(statement) || !t.isStringLiteral(statement.moduleSpecifier)) continue;
      const m = modules.get(statement.moduleSpecifier.text);
      if (!m || statement.importClause?.isTypeOnly) continue;
      m.imports.push(statement.moduleSpecifier);
      const named = statement.importClause?.namedBindings;
      if (named && t.isNamespaceImport(named)) m.namespaces.add(named);
      else if (named)
        for (const element of named.elements)
          if (!element.isTypeOnly && (element.propertyName ?? element.name).text === exported(m))
            m.bindings.add(element);
    }
    const visit = (node: ts.Node): void => {
      if (t.isCallExpression(node)) {
        const spec = node.arguments[0];
        if (node.expression.kind === t.SyntaxKind.ImportKeyword && spec && t.isStringLiteralLike(spec)) {
          modules.get(spec.text)?.imports.push(spec);
        } else {
          const ffi = modules.get(FFI)!;
          const process = modules.get(PROCESS)!;
          if (this.isDlopen(node.expression, ffi)) ffi.calls.push(node);
          else if (this.isSpawn(node.expression, process)) process.calls.push(node);
        }
      }
      t.forEachChild(node, visit);
    };
    visit(file);

    const ffi = modules.get(FFI)!;
    const ffiFirst = ffi.imports[0];
    if (ffiFirst) {
      if (unit.owner)
        report(
          "SK6005",
          `the package \`${unit.owner.name}\` imports \`${FFI}\`: it calls native libraries with this app's permissions`,
          ffiFirst,
        );
      if (!this.allowed.ffi.size && !this.allowed.ffiAddress) {
        report(
          "SK6001",
          `\`${FFI}\` is imported, but sk.config.json has no "permissions.ffi": every dlopen() throws`,
          ffiFirst,
        );
      } else {
        this.names(
          ffi.calls,
          this.allowed.ffi,
          report,
          (value, known) => [
            `\`dlopen(${JSON.stringify(value)})\`: sk.config.json "permissions.ffi" has no entry named ${JSON.stringify(value)}, so the call throws NotAllowedError`,
            `Add { "name": ${JSON.stringify(value)}, "path": "$APP/…" or "$SYSTEM/…" } to "permissions.ffi"${known ? `, or use a listed name: ${known}` : ""}.`,
          ],
          "the library name given to `dlopen()` is not a literal, so it cannot be checked",
        );
      }
    }

    const process = modules.get(PROCESS)!;
    const processFirst = process.imports[0];
    if (processFirst) {
      if (unit.owner)
        report(
          "SK6005",
          `the package \`${unit.owner.name}\` imports \`${PROCESS}\`: it starts programs with this app's permissions`,
          processFirst,
        );
      if (!this.allowed.process.size) {
        report(
          "SK6001",
          `\`${PROCESS}\` is imported, but sk.config.json has no "permissions.process": every Process.spawn() throws`,
          processFirst,
        );
        return;
      }
      for (const name of this.allowed.processWide)
        report(
          "SK6004",
          `"permissions.process" entry ${JSON.stringify(name)} finds its program on PATH or takes any arguments`,
          processFirst,
        );
      this.names(
        process.calls,
        this.allowed.process,
        report,
        (value, known) => [
          `\`Process.spawn(${JSON.stringify(value)})\`: sk.config.json "permissions.process" has no entry named ${JSON.stringify(value)}, so the call throws NotAllowedError`,
          `Add { "name": ${JSON.stringify(value)}, "path": "$APP/…" } to "permissions.process"${known ? `, or use a listed name: ${known}` : ""}.`,
        ],
        "the program given to `Process.spawn()` or `Process.exec()` is not a literal, so it cannot be checked",
        true,
      );
    }
  }

  private names(
    calls: ts.CallExpression[],
    allowed: ReadonlySet<string>,
    report: (code: string, message: string, node: ts.Node, help?: string) => void,
    unlisted: (value: string, known: string) => [string, string],
    notLiteral: string,
    paths = false,
  ): void {
    for (const call of calls) {
      const name = call.arguments[0];
      if (!name) continue;
      const values = this.literals(name);
      if (!values) {
        report("SK6003", notLiteral, name);
        continue;
      }
      for (const value of values) {
        if (allowed.has(value) || (paths && /^([A-Za-z]:)?[\\/]/.test(value))) continue;
        const known = [...allowed].map((n) => JSON.stringify(n)).join(", ");
        const [message, help] = unlisted(value, known);
        report("SK6002", message, name, help);
      }
    }
  }

  private declared(node: ts.Node, among: Set<ts.Node>): boolean {
    return among.size > 0 && (this.checker.getSymbolAtLocation(node)?.declarations ?? []).some((d) => among.has(d));
  }

  private isDlopen(callee: ts.Expression, m: Module): boolean {
    const t = this.typescript;
    if (t.isIdentifier(callee)) return this.declared(callee, m.bindings);
    return (
      t.isPropertyAccessExpression(callee) &&
      callee.name.text === "dlopen" &&
      this.declared(callee.expression, m.namespaces)
    );
  }

  /** `Process.spawn(…)` or `Process.exec(…)`, also as `ns.Process.spawn(…)`. */
  private isSpawn(callee: ts.Expression, m: Module): boolean {
    const t = this.typescript;
    if (!t.isPropertyAccessExpression(callee) || !["spawn", "exec"].includes(callee.name.text)) return false;
    const target = callee.expression;
    if (t.isIdentifier(target)) return this.declared(target, m.bindings);
    return (
      t.isPropertyAccessExpression(target) &&
      target.name.text === "Process" &&
      this.declared(target.expression, m.namespaces)
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
