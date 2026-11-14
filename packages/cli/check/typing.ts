import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import type ts from "typescript";
import { nodeRange, ZERO_RANGE, type Finding, type FindingRelated } from "./finding";
import type { PackageInfo, Packages } from "./packages";
import type { TypeScript, Unit } from "./program";

export interface Score {
  owner: PackageInfo | null;
  declarations: number;
  any: number;
}

interface Hit {
  node: ts.Node;
  what: string;
}

interface Weak {
  declaration: ts.Declaration;
  hits: Hit[];
}

const MAX_DEPTH = 3;
/** SK3002 fires from this share of `any` declarations: a few deliberate catch-alls in a well-typed package are not worth a warning. */
export const WEAK_SHARE = 0.15;
const MAX_RELATED = 5;
const JS_FILE = /\.[cm]?jsx?$/i;
const JSDOC_TYPES = /\/\*\*[\s\S]*?@(?:type|typedef|param|returns?|callback|template)\b/;
const DIRECTIVE = /^(?:\/\/\/?|\/\*+)\s*@ts-(nocheck|ignore|expect-error)\b/;

function hasTypesCondition(exports: unknown): boolean {
  if (!exports || typeof exports !== "object") return false;
  return Object.entries(exports as Record<string, unknown>).some(
    ([key, value]) => key === "types" || key === "typings" || hasTypesCondition(value),
  );
}

const typesPackageName = (name: string) => (name.startsWith("@") ? name.slice(1).replace("/", "__") : name);

export class Typing {
  private readonly scores = new Map<string, Score>();
  private readonly weak = new Map<string, Weak[]>();
  private readonly untyped = new Set<string>();
  private readonly byProgramPath = new Map<string, Unit>();
  private readonly caseless: boolean;

  constructor(
    private readonly typescript: TypeScript,
    private readonly program: ts.Program,
    private readonly checker: ts.TypeChecker,
    private readonly packages: Packages,
    private readonly units: Unit[],
  ) {
    this.caseless = !typescript.sys.useCaseSensitiveFileNames;
    for (const unit of units) this.byProgramPath.set(this.key(unit.programPath), unit);
  }

  private key(path: string): string {
    return this.caseless ? resolve(path).toLowerCase() : resolve(path);
  }

  private realPath(file: ts.SourceFile): string {
    return this.byProgramPath.get(this.key(file.fileName))?.path ?? resolve(file.fileName);
  }

  private ownerOf(file: ts.SourceFile): PackageInfo | null {
    return this.packages.ownerOf(this.realPath(file));
  }

  private score(owner: PackageInfo | null): Score {
    const id = owner ? `${owner.name}@${owner.version}` : "";
    let score = this.scores.get(id);
    if (!score) this.scores.set(id, (score = { owner, declarations: 0, any: 0 }));
    return score;
  }

  /** SK3001: packages with code in the bundle and no declarations anywhere. */
  untypedPackages(out: Finding[]): void {
    const byPackage = new Map<string, { owner: PackageInfo; units: Unit[] }>();
    for (const unit of this.units) {
      if (!unit.owner) continue;
      const id = `${unit.owner.name}@${unit.owner.version}`;
      const entry = byPackage.get(id) ?? { owner: unit.owner, units: [] };
      entry.units.push(unit);
      byPackage.set(id, entry);
    }
    for (const [id, { owner, units }] of byPackage) {
      if (this.typed(owner, units)) continue;
      this.untyped.add(id);
      out.push({
        code: "SK3001",
        message: `\`${owner.name}@${owner.version}\` ships no type declarations; its ${units.length} module${units.length === 1 ? "" : "s"} in the bundle ${units.length === 1 ? "is" : "are"} compiled untyped`,
        file: join(owner.dir, "package.json"),
        range: ZERO_RANGE,
        help: `Install @types/${typesPackageName(owner.name)} if it exists, or prefer a typed alternative.`,
        related: [],
      });
    }
  }

  private typed(owner: PackageInfo, units: Unit[]): boolean {
    const json = owner.json;
    if (typeof json.types === "string" || typeof json.typings === "string" || hasTypesCondition(json.exports))
      return true;
    if (units.some((u) => u.typed)) return true;
    if (existsSync(join(owner.dir, "index.d.ts"))) return true;
    for (const unit of units) {
      const base = unit.path.replace(JS_FILE, "");
      const ext = /\.([cm]?)jsx?$/i.exec(unit.path)?.[1] ?? "";
      if (existsSync(`${base}.d.${ext}ts`) || existsSync(`${base}.d.ts`)) return true;
    }
    const types = join("node_modules", "@types", typesPackageName(owner.name), "package.json");
    for (const start of [owner.dir, this.packages.appRoot]) {
      for (let dir = start; ; dir = dirname(dir)) {
        if (existsSync(join(dir, types))) return true;
        if (dirname(dir) === dir) break;
      }
    }
    return false;
  }

  /** SK3002 and SK3004: what the app imports, and how well it is typed. */
  imports(out: Finding[]): void {
    const t = this.typescript;
    const shorthand = new Map<ts.ModuleDeclaration, FindingRelated[]>();
    const plainJs = new Map<ts.SourceFile, FindingRelated[]>();
    const queue: { declaration: ts.Declaration; depth: number }[] = [];
    const untypedImports = new Map<string, Set<string>>();
    const importSites = new Map<string, FindingRelated[]>();

    const enqueueSymbol = (symbol: ts.Symbol | undefined, depth: number) => {
      if (!symbol) return;
      const target = symbol.flags & t.SymbolFlags.Alias ? this.safeAliased(symbol) : symbol;
      for (const declaration of target?.declarations ?? []) queue.push({ declaration, depth });
    };

    for (const unit of this.units) {
      if (unit.owner || !unit.sourceFile) continue;
      const file = unit.sourceFile;
      const site = (node: ts.Node): FindingRelated => ({
        file: unit.path,
        range: nodeRange(file, node),
        message: "imported here",
      });
      const visit = (node: ts.Node): void => {
        let specifier: ts.Expression | undefined;
        if (t.isImportDeclaration(node) || t.isExportDeclaration(node)) specifier = node.moduleSpecifier;
        else if (t.isImportEqualsDeclaration(node) && t.isExternalModuleReference(node.moduleReference))
          specifier = node.moduleReference.expression;
        else if (
          t.isCallExpression(node) &&
          (node.expression.kind === t.SyntaxKind.ImportKeyword ||
            (t.isIdentifier(node.expression) && node.expression.text === "require"))
        )
          specifier = node.arguments[0];
        if (specifier && t.isStringLiteralLike(specifier)) {
          const moduleSymbol = this.checker.getSymbolAtLocation(specifier);
          const declaration = moduleSymbol?.declarations?.[0];
          if (moduleSymbol && declaration) {
            if (t.isModuleDeclaration(declaration) && t.isStringLiteral(declaration.name)) {
              if (!declaration.body || declaration.name.text === "*") {
                const list = shorthand.get(declaration) ?? [];
                list.push(site(specifier));
                shorthand.set(declaration, list);
              }
            } else if (t.isSourceFile(declaration)) {
              const owner = this.ownerOf(declaration);
              const isJs = !declaration.isDeclarationFile && JS_FILE.test(declaration.fileName);
              if (!owner) {
                if (
                  isJs &&
                  !/[\\/]node_modules[\\/]/.test(declaration.fileName) &&
                  !JSDOC_TYPES.test(declaration.text)
                ) {
                  const list = plainJs.get(declaration) ?? [];
                  list.push(site(specifier));
                  plainJs.set(declaration, list);
                }
              } else if (isJs) {
                const id = `${owner.name}@${owner.version}`;
                const names = untypedImports.get(id) ?? new Set<string>();
                for (const name of this.importedNames(node)) names.add(name);
                untypedImports.set(id, names);
                importSites.set(id, [...(importSites.get(id) ?? []), site(specifier)]);
                this.score(owner);
              } else {
                this.score(owner);
                for (const symbol of this.importedSymbols(node, moduleSymbol, file)) enqueueSymbol(symbol, 0);
              }
            }
          }
        }
        t.forEachChild(node, visit);
      };
      visit(file);
    }

    for (const [id, names] of untypedImports) {
      const score = this.scores.get(id);
      if (!score?.owner) continue;
      score.declarations += Math.max(names.size, 1);
      score.any += Math.max(names.size, 1);
      const owner = score.owner;
      if (
        !this.typed(
          owner,
          this.units.filter((u) => u.owner?.dir === owner.dir),
        )
      )
        continue;
      // It ships declarations, but its "exports" (or main) lead TypeScript to the JavaScript.
      out.push({
        code: "SK3004",
        message: `\`${id}\` ships type declarations, but its imports resolve to the JavaScript (for example through an "exports" map without a "types" condition), so they are \`any\``,
        file: join(owner.dir, "package.json"),
        range: ZERO_RANGE,
        help: 'Ask the package to add a "types" condition to its "exports"; meanwhile a local declare module with real types restores them.',
        related: importSites.get(id) ?? [],
      });
    }

    const seen = new Set<ts.Node>();
    while (queue.length) {
      const { declaration, depth } = queue.pop()!;
      if (seen.has(declaration)) continue;
      seen.add(declaration);
      const file = declaration.getSourceFile();
      if (this.program.isSourceFileDefaultLibrary(file)) continue;
      if (!file.isDeclarationFile && JS_FILE.test(file.fileName)) continue;
      const owner = this.ownerOf(file);
      if (!owner) continue;
      const score = this.score(owner);
      const hits: Hit[] = [];
      const follow = (symbol: ts.Symbol | undefined) => {
        if (depth < MAX_DEPTH) enqueueSymbol(symbol, depth + 1);
      };
      if (this.declarationHits(declaration, hits, follow)) {
        score.declarations++;
        if (hits.length) {
          score.any++;
          const id = `${owner.name}@${owner.version}`;
          const list = this.weak.get(id) ?? [];
          list.push({ declaration, hits });
          this.weak.set(id, list);
        }
      }
      if (depth < MAX_DEPTH)
        for (const member of this.members(declaration)) queue.push({ declaration: member, depth: depth + 1 });
    }

    for (const [id, score] of this.scores) {
      if (!score.owner || this.untyped.has(id) || score.any < score.declarations * WEAK_SHARE) continue;
      const list = (this.weak.get(id) ?? []).sort((a, b) => b.hits.length - a.hits.length);
      const first = list[0];
      if (!first) continue;
      const related = list.slice(0, MAX_RELATED).map((w) => this.weakRelated(w));
      out.push({
        code: "SK3002",
        message: `\`${score.owner.name}@${score.owner.version}\`: ${score.any} of ${score.declarations} exported declarations the app reaches use \`any\``,
        file: related[0]!.file,
        range: related[0]!.range,
        related,
      });
    }

    for (const [declaration, sites] of shorthand) {
      const file = declaration.getSourceFile();
      const name = (declaration.name as ts.StringLiteral).text;
      out.push({
        code: "SK3004",
        message:
          name === "*"
            ? '`declare module "*"` makes every unresolved import `any`'
            : `\`declare module "${name}"\` has no body, so everything imported from it is \`any\``,
        file: this.realPath(file),
        range: nodeRange(file, declaration),
        help: "Remove the shorthand declaration so the package's own types apply, or give it a body with real types.",
        related: sites,
      });
    }
    for (const [file, sites] of plainJs) {
      out.push({
        code: "SK3004",
        message: `this JavaScript file has no JSDoc types, so what ${sites.length === 1 ? "an import takes" : `${sites.length} imports take`} from it is \`any\``,
        file: this.realPath(file),
        range: ZERO_RANGE,
        help: "Convert the file to TypeScript, or annotate its exports with JSDoc types.",
        related: sites,
      });
    }
  }

  private safeAliased(symbol: ts.Symbol): ts.Symbol | undefined {
    try {
      return this.checker.getAliasedSymbol(symbol);
    } catch {
      return undefined;
    }
  }

  private importedNames(node: ts.Node): string[] {
    const t = this.typescript;
    if (t.isImportDeclaration(node) && node.importClause) {
      const clause = node.importClause;
      const names = clause.name ? ["default"] : [];
      if (clause.namedBindings && t.isNamedImports(clause.namedBindings))
        names.push(...clause.namedBindings.elements.map((e) => (e.propertyName ?? e.name).text));
      else if (clause.namedBindings) names.push("*");
      return names;
    }
    if (t.isExportDeclaration(node) && node.exportClause && t.isNamedExports(node.exportClause))
      return node.exportClause.elements.map((e) => (e.propertyName ?? e.name).text);
    return ["*"];
  }

  private importedSymbols(node: ts.Node, moduleSymbol: ts.Symbol, file: ts.SourceFile): ts.Symbol[] {
    const t = this.typescript;
    const symbols: (ts.Symbol | undefined)[] = [];
    const exportsOf = () => {
      try {
        return this.checker.getExportsOfModule(moduleSymbol);
      } catch {
        return [];
      }
    };
    if (t.isImportDeclaration(node)) {
      const clause = node.importClause;
      if (!clause) return [];
      if (clause.name) symbols.push(this.checker.getSymbolAtLocation(clause.name));
      const bindings = clause.namedBindings;
      if (bindings && t.isNamedImports(bindings))
        for (const element of bindings.elements) symbols.push(this.checker.getSymbolAtLocation(element.name));
      else if (bindings && t.isNamespaceImport(bindings)) {
        const namespace = this.checker.getSymbolAtLocation(bindings.name);
        const used: ts.Symbol[] = [];
        const look = (n: ts.Node): void => {
          if (
            t.isPropertyAccessExpression(n) &&
            t.isIdentifier(n.expression) &&
            this.checker.getSymbolAtLocation(n.expression) === namespace
          ) {
            const member = this.checker.getSymbolAtLocation(n.name);
            if (member) used.push(member);
          }
          t.forEachChild(n, look);
        };
        look(file);
        symbols.push(...(used.length ? used : exportsOf()));
      }
    } else if (t.isExportDeclaration(node)) {
      if (node.exportClause && t.isNamedExports(node.exportClause))
        for (const element of node.exportClause.elements)
          symbols.push(this.checker.getExportSpecifierLocalTargetSymbol(element));
    } else symbols.push(...exportsOf());
    return symbols.filter((s): s is ts.Symbol => !!s);
  }

  private members(declaration: ts.Declaration): ts.Declaration[] {
    const t = this.typescript;
    if (t.isInterfaceDeclaration(declaration) || t.isClassDeclaration(declaration) || t.isTypeLiteralNode(declaration))
      return [...declaration.members].filter(
        (m) => !(t.isPropertyDeclaration(m) && m.modifiers?.some((x) => x.kind === t.SyntaxKind.PrivateKeyword)),
      );
    if (t.isModuleDeclaration(declaration) && declaration.body && t.isModuleBlock(declaration.body)) {
      const out: ts.Declaration[] = [];
      for (const statement of declaration.body.statements) {
        if (t.isVariableStatement(statement)) out.push(...statement.declarationList.declarations);
        else if (
          t.isFunctionDeclaration(statement) ||
          t.isClassDeclaration(statement) ||
          t.isInterfaceDeclaration(statement) ||
          t.isTypeAliasDeclaration(statement)
        )
          out.push(statement);
      }
      return out;
    }
    return [];
  }

  /** Collects the `any`s of one declaration's own signature; false when it is not a declaration that counts. */
  private declarationHits(
    declaration: ts.Declaration,
    hits: Hit[],
    follow: (symbol: ts.Symbol | undefined) => void,
  ): boolean {
    const t = this.typescript;
    if (t.getNameOfDeclaration(declaration)?.getText().startsWith("__VLS_")) return false;
    const file = declaration.getSourceFile();
    const ambient = file.isDeclarationFile;
    const scan = (type: ts.TypeNode | undefined) => {
      if (type) this.scanType(type, hits, follow);
    };
    const untypedValue = (node: ts.Node, what: string) => {
      if (ambient) hits.push({ node, what });
      else if (this.isAny(node)) hits.push({ node, what });
    };
    if (
      t.isFunctionDeclaration(declaration) ||
      t.isMethodDeclaration(declaration) ||
      t.isMethodSignature(declaration) ||
      t.isCallSignatureDeclaration(declaration) ||
      t.isConstructSignatureDeclaration(declaration) ||
      t.isConstructorDeclaration(declaration) ||
      t.isIndexSignatureDeclaration(declaration) ||
      t.isGetAccessorDeclaration(declaration) ||
      t.isSetAccessorDeclaration(declaration)
    ) {
      for (const parameter of declaration.parameters) {
        if (t.isIdentifier(parameter.name) && parameter.name.text === "this") {
          scan(parameter.type);
          continue;
        }
        if (parameter.type) scan(parameter.type);
        else untypedValue(parameter, "an untyped parameter");
      }
      if (declaration.type) scan(declaration.type);
      else if (!t.isConstructorDeclaration(declaration) && !t.isSetAccessorDeclaration(declaration)) {
        if (ambient) hits.push({ node: declaration, what: "no return type" });
        else {
          const signature = this.checker.getSignatureFromDeclaration(declaration as ts.SignatureDeclaration);
          if (signature && this.checker.getReturnTypeOfSignature(signature).flags & t.TypeFlags.Any)
            hits.push({ node: declaration, what: "an `any` result" });
        }
      }
      return true;
    }
    if (
      t.isVariableDeclaration(declaration) ||
      t.isPropertySignature(declaration) ||
      t.isPropertyDeclaration(declaration)
    ) {
      if (declaration.type) scan(declaration.type);
      else if (!(t.isVariableDeclaration(declaration) && declaration.initializer && ambient))
        untypedValue(declaration.name, "no type");
      return true;
    }
    if (t.isTypeAliasDeclaration(declaration)) {
      scan(declaration.type);
      return true;
    }
    if (t.isInterfaceDeclaration(declaration) || t.isClassDeclaration(declaration)) {
      for (const clause of declaration.heritageClauses ?? []) for (const type of clause.types) scan(type);
      return true;
    }
    return false;
  }

  private isAny(node: ts.Node): boolean {
    try {
      return (this.checker.getTypeAtLocation(node).flags & this.typescript.TypeFlags.Any) !== 0;
    } catch {
      return false;
    }
  }

  /** `any`, and the catch-alls `Function`, `Object` and `{}`, outside type-level machinery (constraints, conditions). */
  private scanType(root: ts.Node, hits: Hit[], follow: (symbol: ts.Symbol | undefined) => void): void {
    const t = this.typescript;
    /** `=> any` of a callback the caller passes in: its result is not used as a value here. */
    const callbackResult = (node: ts.Node) => {
      if (!(t.isFunctionTypeNode(node.parent) || t.isConstructorTypeNode(node.parent)) || node.parent.type !== node)
        return false;
      for (let n: ts.Node = node.parent; n !== root && n.parent; n = n.parent) if (t.isParameter(n)) return true;
      return t.isParameter(root.parent);
    };
    /** The type of a value as a whole (or a union member of it), not a piece of type-level machinery. */
    const direct = (node: ts.Node) => {
      let n = node;
      while (
        t.isUnionTypeNode(n.parent) ||
        t.isParenthesizedTypeNode(n.parent) ||
        t.isArrayTypeNode(n.parent) ||
        (t.isTypeOperatorNode(n.parent) && n.parent.operator === t.SyntaxKind.ReadonlyKeyword)
      )
        n = n.parent;
      const p = n.parent;
      return (
        (t.isParameter(p) ||
          t.isPropertySignature(p) ||
          t.isPropertyDeclaration(p) ||
          t.isVariableDeclaration(p) ||
          t.isIndexSignatureDeclaration(p) ||
          t.isFunctionTypeNode(p) ||
          t.isMethodSignature(p) ||
          t.isMethodDeclaration(p) ||
          t.isFunctionDeclaration(p) ||
          t.isCallSignatureDeclaration(p)) &&
        "type" in p &&
        p.type === n
      );
    };
    const visit = (node: ts.Node): void => {
      if (node.kind === t.SyntaxKind.AnyKeyword) {
        if (!callbackResult(node)) hits.push({ node, what: "`any`" });
        return;
      }
      if (t.isTypeParameterDeclaration(node) || t.isInferTypeNode(node)) return;
      if (t.isTypeOperatorNode(node) && node.operator === t.SyntaxKind.KeyOfKeyword) return;
      if (t.isConditionalTypeNode(node)) {
        visit(node.trueType);
        visit(node.falseType);
        return;
      }
      if (t.isTypeLiteralNode(node) && node.members.length === 0) {
        if (direct(node)) hits.push({ node, what: "`{}`" });
        return;
      }
      if (t.isTypeReferenceNode(node) || t.isExpressionWithTypeArguments(node) || t.isImportTypeNode(node)) {
        const name = t.isTypeReferenceNode(node)
          ? node.typeName
          : t.isImportTypeNode(node)
            ? node.qualifier
            : node.expression;
        if (!name) return;
        const symbol = this.symbolOf(t.isQualifiedName(name) ? name.right : name);
        const text = t.isIdentifier(name) ? name.text : t.isQualifiedName(name) ? name.right.text : "";
        // vue-tsc's component declarations: the arguments of DefineComponent and its __VLS_ helpers
        // are the template's plumbing (slot signatures, emits maps), not values the app's code reads.
        if (text === "DefineComponent" || text.startsWith("__VLS_")) return;
        const fromLib = symbol?.declarations?.some((d) => this.program.isSourceFileDefaultLibrary(d.getSourceFile()));
        if ((text === "Function" || text === "Object") && fromLib && direct(node)) {
          hits.push({ node, what: `\`${text}\`` });
          return;
        }
        if (!fromLib) follow(symbol);
        const target = symbol && symbol.flags & t.SymbolFlags.Alias ? this.safeAliased(symbol) : symbol;
        const parameters = target?.declarations
          ?.map((d) =>
            "typeParameters" in d ? (d.typeParameters as ts.NodeArray<ts.TypeParameterDeclaration>) : undefined,
          )
          .find((list) => list?.length);
        node.typeArguments?.forEach((argument, i) => {
          // `any` spelled where the parameter defaults to `any` anyway (`FunctionalComponent<P, {}, any>`).
          if (argument.kind === t.SyntaxKind.AnyKeyword && parameters?.[i]?.default?.kind === t.SyntaxKind.AnyKeyword)
            return;
          visit(argument);
        });
        return;
      }
      if (t.isTypeQueryNode(node)) {
        follow(this.symbolOf(node.exprName));
        return;
      }
      if (t.isParameter(node) && !node.type && node.getSourceFile().isDeclarationFile) {
        hits.push({ node, what: "an untyped parameter" });
        return;
      }
      t.forEachChild(node, visit);
    };
    visit(root);
  }

  private symbolOf(node: ts.Node): ts.Symbol | undefined {
    try {
      return this.checker.getSymbolAtLocation(node);
    } catch {
      return undefined;
    }
  }

  private weakRelated(weak: Weak): FindingRelated {
    const t = this.typescript;
    const file = weak.declaration.getSourceFile();
    const name = t.getNameOfDeclaration(weak.declaration);
    const parent = weak.declaration.parent;
    const parentName =
      parent && (t.isInterfaceDeclaration(parent) || t.isClassDeclaration(parent) || t.isTypeAliasDeclaration(parent))
        ? parent.name?.text
        : undefined;
    const own = name
      ? name.getText(file)
      : t.isIndexSignatureDeclaration(weak.declaration)
        ? "[index]"
        : t.isCallSignatureDeclaration(weak.declaration)
          ? "(call)"
          : t.isConstructSignatureDeclaration(weak.declaration) || t.isConstructorDeclaration(weak.declaration)
            ? "constructor"
            : "a declaration";
    const label = parentName ? `${parentName}${own.startsWith("[") || own.startsWith("(") ? "" : "."}${own}` : own;
    const kinds = [...new Set(weak.hits.map((h) => h.what))].join(", ");
    return {
      file: this.realPath(file),
      range: nodeRange(file, weak.hits[0]!.node),
      message: `${label}: ${weak.hits.length} × ${kinds}`,
    };
  }

  /** SK3003 and the app's own score. */
  appCode(out: Finding[]): void {
    const t = this.typescript;
    const score = this.score(null);
    for (const unit of this.units) {
      if (unit.owner || !unit.sourceFile) continue;
      const file = unit.sourceFile;
      const hits: Hit[] = [];
      const comments = new Set<number>();
      const directives = (pos: number) => {
        for (const range of [
          ...(t.getLeadingCommentRanges(file.text, pos) ?? []),
          ...(t.getTrailingCommentRanges(file.text, pos) ?? []),
        ]) {
          if (comments.has(range.pos)) continue;
          comments.add(range.pos);
          const match = DIRECTIVE.exec(file.text.slice(range.pos, range.end));
          if (match) hits.push({ node: { pos: range.pos, end: range.end } as ts.Node, what: `\`@ts-${match[1]}\`` });
        }
      };
      const countValue = (node: ts.Node) => {
        score.declarations++;
        if (this.isAny(node)) score.any++;
      };
      const visit = (node: ts.Node, typeLevel: boolean): void => {
        directives(node.pos);
        if (unit.typed && node.kind === t.SyntaxKind.AnyKeyword && !typeLevel) {
          const assertion = t.isAsExpression(node.parent) || t.isTypeAssertionExpression(node.parent);
          hits.push({ node, what: assertion ? "`as any`" : "`any`" });
        }
        if (t.isParameter(node) && !(t.isIdentifier(node.name) && node.name.text === "this")) {
          score.declarations++;
          const any = this.isAny(node.name);
          if (any) score.any++;
          if (unit.typed && any && !node.type && !node.initializer && !this.inTypeOnly(node) && !this.contextual(node))
            hits.push({ node: node.name, what: "an implicitly `any` parameter" });
        } else if (t.isVariableDeclaration(node) && t.isIdentifier(node.name)) countValue(node.name);
        else if ((t.isPropertyDeclaration(node) || t.isPropertySignature(node)) && node.name) countValue(node.name);
        else if ((t.isFunctionDeclaration(node) || t.isMethodDeclaration(node)) && node.body) {
          score.declarations++;
          const signature = this.checker.getSignatureFromDeclaration(node);
          if (signature && this.checker.getReturnTypeOfSignature(signature).flags & t.TypeFlags.Any) score.any++;
        }
        const level =
          typeLevel ||
          t.isTypeParameterDeclaration(node) ||
          t.isInferTypeNode(node) ||
          (t.isConditionalTypeNode(node) && false);
        if (t.isConditionalTypeNode(node)) {
          visit(node.checkType, true);
          visit(node.extendsType, true);
          visit(node.trueType, level);
          visit(node.falseType, level);
          return;
        }
        t.forEachChild(node, (child) => visit(child, level));
      };
      visit(file, false);
      directives(file.endOfFileToken.pos);
      if (!hits.length) continue;
      hits.sort((a, b) => a.node.pos - b.node.pos);
      const range = (hit: Hit) =>
        hit.node.parent === undefined && !("kind" in hit.node && hit.node.kind)
          ? {
              start: file.getLineAndCharacterOfPosition(hit.node.pos),
              end: file.getLineAndCharacterOfPosition(hit.node.end),
            }
          : nodeRange(file, hit.node);
      const counts = new Map<string, number>();
      for (const hit of hits) counts.set(hit.what, (counts.get(hit.what) ?? 0) + 1);
      const summary = [...counts].map(([what, n]) => `${n} × ${what}`).join(", ");
      out.push({
        code: "SK3003",
        message: `${hits.length} untyped place${hits.length === 1 ? "" : "s"} in this file: ${summary}`,
        file: unit.path,
        range: range(hits[0]!),
        related: hits.map((hit) => ({ file: unit.path, range: range(hit), message: hit.what })),
      });
    }
  }

  /** The parameter's function gets its type from where it is passed (`.catch((e) => …)`): any `any` comes from there. */
  private contextual(parameter: ts.ParameterDeclaration): boolean {
    const t = this.typescript;
    const fn = parameter.parent;
    try {
      if (t.isArrowFunction(fn) || t.isFunctionExpression(fn)) return this.checker.getContextualType(fn) !== undefined;
      if (t.isMethodDeclaration(fn) && t.isObjectLiteralExpression(fn.parent))
        return this.checker.getContextualType(fn.parent) !== undefined;
    } catch {}
    return false;
  }

  /** A parameter of a signature that has no body (an overload, a function type): no value flows through it here. */
  private inTypeOnly(parameter: ts.ParameterDeclaration): boolean {
    const t = this.typescript;
    const fn = parameter.parent;
    return (
      !(
        t.isFunctionDeclaration(fn) ||
        t.isMethodDeclaration(fn) ||
        t.isArrowFunction(fn) ||
        t.isFunctionExpression(fn) ||
        t.isConstructorDeclaration(fn) ||
        t.isGetAccessorDeclaration(fn) ||
        t.isSetAccessorDeclaration(fn)
      ) || !("body" in fn && fn.body)
    );
  }

  results(): Score[] {
    return [...this.scores.values()];
  }
}
