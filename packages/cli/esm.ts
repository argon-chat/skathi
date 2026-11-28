import { isAbsolute, join } from "node:path";
import ts from "typescript";

interface Reference {
  start: number;
  end: number;
  spec: string;
  names: string[];
  dynamic: boolean;
  argument?: { start: number; end: number };
}

/** Source ranges in the emitted JavaScript, including quoted and re-exported module names. */
export function moduleReferences(source: string): Reference[] {
  const file = ts.createSourceFile("module.mjs", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const references: Reference[] = [];
  function visit(node: ts.Node) {
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
      const spec = node.moduleSpecifier;
      if (!spec || !ts.isStringLiteral(spec)) return;
      const clause = ts.isImportDeclaration(node) ? node.importClause?.namedBindings : node.exportClause;
      const names =
        clause && (ts.isNamedImports(clause) || ts.isNamedExports(clause))
          ? clause.elements.map((item) => (item.propertyName ?? item.name).text)
          : [];
      references.push({ start: spec.getStart(file), end: spec.end, spec: spec.text, names, dynamic: false });
      return;
    }
    if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
      const spec = node.arguments[0];
      references.push({
        start: node.expression.getStart(file),
        end: node.expression.end,
        spec: spec && ts.isStringLiteralLike(spec) ? spec.text : "",
        names: [],
        dynamic: true,
        ...(spec && ts.isStringLiteralLike(spec) && { argument: { start: spec.getStart(file), end: spec.end } }),
      });
    }
    ts.forEachChild(node, visit);
  }
  visit(file);
  return references;
}

export function defineModule(id: string, body: string, refs = moduleReferences(body)): string {
  return (
    "__SK__.defineModule(" +
    [id, body, refs, Bun.hash(body).toString(16)].map((part) => JSON.stringify(part)).join(", ") +
    ");"
  );
}

/** Bun 1.4 emits null for module-scope this. Preserve the ESM value before Bun folds expressions. */
export function normalizeModuleThis(source: string, filename: string): string | null {
  if (!/\bthis\b/.test(source) || /(?:^|[/\\])node_modules[/\\]|\.c[jt]s$/.test(filename)) return null;
  const file = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, false);
  const positions: number[] = [];
  function visit(node: ts.Node): void {
    if (node.kind === ts.SyntaxKind.ThisKeyword) {
      positions.push(node.getStart(file));
      return;
    }
    if (ts.isTypeNode(node)) return;
    if (
      (ts.isFunctionLike(node) && !ts.isArrowFunction(node)) ||
      ts.isPropertyDeclaration(node) ||
      ts.isClassStaticBlockDeclaration(node)
    ) {
      // Computed member names and decorators execute outside the method or field initializer.
      if ("name" in node && node.name && ts.isComputedPropertyName(node.name)) visit(node.name.expression);
      if (ts.canHaveDecorators(node)) for (const decorator of ts.getDecorators(node) ?? []) visit(decorator.expression);
      return;
    }
    ts.forEachChild(node, visit);
  }
  visit(file);
  if (!positions.length) return null;
  let end = 0;
  const parts = positions
    .sort((a, b) => a - b)
    .map((start) => {
      const part = source.slice(end, start) + "(void 0)";
      end = start + 4;
      return part;
    });
  return parts.join("") + source.slice(end);
}

/** Bundlers may absorb legacy npm dependencies. Application sources must be ES modules. */
export function appModuleError(id: string, format: string | undefined): string | null {
  if (
    format !== "cjs" ||
    id.includes("\0") ||
    id.startsWith("sk-native:native:") ||
    /(?:^|[/\\])node_modules[/\\]/.test(id)
  )
    return null;
  return id + ": Skathi supports only ES modules (ESM). Use import/export in application code.";
}

export function checkAppModules(inputs: Record<string, { format?: string }>): void {
  for (const [id, input] of Object.entries(inputs)) {
    const error = appModuleError(id, input.format);
    if (error) throw new Error(error);
  }
}

export async function browserImportEntry(
  spec: string,
  from: string,
  conditions: string[] = [],
): Promise<string | null> {
  const probe = join(from, "__sk_resolve_probe__.js"); // never written: onLoad answers for it
  let hit: string | null = null;
  await Bun.build({
    entrypoints: [probe],
    target: "browser",
    conditions,
    format: "esm",
    plugins: [
      {
        name: "sk-resolve-probe",
        setup(b: Bun.PluginBuilder) {
          b.onResolve({ filter: /__sk_resolve_probe__\.js$/ }, () => ({ path: probe }));
          b.onLoad({ filter: /.*/ }, (args: Bun.OnLoadArgs) => {
            if (args.path === probe) {
              return { contents: "import " + JSON.stringify(spec) + ";", loader: "js" };
            }
            hit ??= args.path;
            return { contents: "", loader: "js" };
          });
        },
      },
    ],
  }).catch(() => null);
  return hit && isAbsolute(hit) ? hit : null;
}
