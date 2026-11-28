// Reads the same TypeScript declarations an app compiles against. This runs in the docs build process.
import { dirname, relative, resolve } from "node:path";
import ts from "typescript";
import { workspaceRoot } from "./workspace";

export interface Declaration {
  name: string;
  source: string;
  summary: string;
  syntax: string;
}

const format = ts.TypeFormatFlags.NoTruncation;
const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed, removeComments: true });
const withDocs = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed });
const sorted = (symbols: ts.Symbol[]) => symbols.sort((a, b) => a.name.localeCompare(b.name, "en"));

function hidden(node?: ts.Node) {
  if (!node) return false;
  return (
    (ts.isClassElement(node) && node.name !== undefined && ts.isPrivateIdentifier(node.name)) ||
    ts.getJSDocTags(node).some((tag) => tag.tagName.text === "internal") ||
    (ts.canHaveModifiers(node) &&
      ts
        .getModifiers(node)
        ?.some((m) => m.kind === ts.SyntaxKind.PrivateKeyword || m.kind === ts.SyntaxKind.ProtectedKeyword))
  );
}

function loadBindings() {
  const config = resolve(workspaceRoot, "packages/runtime/tsconfig.app.json");
  const diagnostics = (d: ts.Diagnostic) => ts.flattenDiagnosticMessageText(d.messageText, "\n");
  const parsed = ts.getParsedCommandLineOfConfigFile(
    config,
    {},
    {
      ...ts.sys,
      onUnRecoverableConfigFileDiagnostic(d) {
        throw new Error(diagnostics(d));
      },
    },
  );
  if (!parsed || parsed.errors.length) throw new Error(parsed?.errors.map(diagnostics).join("\n") ?? config);
  const runtime = dirname(config);
  const registryFile = resolve(runtime, "src/std/index.ts");
  const program = ts.createProgram([...parsed.fileNames, registryFile], parsed.options);
  const checker = program.getTypeChecker();
  const sdk = resolve(runtime, "../..");
  const source = (node: ts.Node) => relative(sdk, node.getSourceFile().fileName).replaceAll("\\", "/");
  function print(node: ts.Node, docs: boolean, summary = "") {
    const source = node.getSourceFile();
    if (ts.isInterfaceDeclaration(node) && node.members.some(hidden)) {
      node = ts.factory.updateInterfaceDeclaration(
        node,
        node.modifiers,
        node.name,
        node.typeParameters,
        node.heritageClauses,
        node.members.filter((member) => !hidden(member)),
      );
    }
    const text = (docs ? withDocs : printer).printNode(ts.EmitHint.Unspecified, node, source);
    if (!docs || !summary) return text;
    const headline = summary.replace(/\s+/g, " ").trim();
    for (const range of ts.getLeadingCommentRanges(text, 0) ?? []) {
      const comment = text.slice(range.pos, range.end);
      if (!comment.startsWith("/**")) continue;
      const prose = comment
        .slice(3, -2)
        .replace(/^\s*\* ?/gm, "")
        .replace(/\s+/g, " ")
        .trim();
      if (prose === headline) return text.slice(0, range.pos) + text.slice(range.end).trimStart();
    }
    return text;
  }
  const std = new Map<string, Map<string, ts.Symbol>>();
  for (const module of checker.getAmbientModules()) {
    const name = module.name.replace(/^"|"$/g, "");
    if (!name.startsWith("skathi:")) continue;
    const exports = sorted(
      checker.getExportsOfModule(module).filter((s) => !resolveSymbol(s).declarations?.every(hidden)),
    );
    if (!exports.length) throw new Error(`Cannot resolve exports of ${name} using ${config}`);
    std.set(name, new Map(exports.map((symbol) => [symbol.name, symbol])));
  }
  const registry = program
    .getSourceFile(registryFile)!
    .statements.filter(ts.isVariableStatement)
    .flatMap((statement) => [...statement.declarationList.declarations])
    .find((d) => ts.isIdentifier(d.name) && d.name.text === "stdModules")?.initializer;
  if (!registry) throw new Error("Missing stdModules in the dev loader's registry.");
  const moduleSummaries: ReadonlyMap<string, string> = new Map(
    checker
      .getTypeAtLocation(registry)
      .getProperties()
      .map((s) => [`skathi:${s.name}`, ts.displayPartsToString(s.getDocumentationComment(checker))]),
  );
  for (const name of new Set([...moduleSummaries.keys(), ...std.keys()])) {
    if (moduleSummaries.has(name) !== std.has(name))
      throw new Error(`${name} must exist in both stdModules and the public skathi:* declarations.`);
  }

  // The other app declarations describe the host's private __sk_* protocol and module loaders.
  const web = new Map<string, ts.Symbol>();
  function collect(node: ts.Node) {
    if (
      ts.isSourceFile(node) ||
      ts.isModuleBlock(node) ||
      ts.isVariableStatement(node) ||
      ts.isVariableDeclarationList(node)
    ) {
      ts.forEachChild(node, collect);
    } else if (ts.isModuleDeclaration(node) && node.flags & ts.NodeFlags.GlobalAugmentation && node.body) {
      collect(node.body);
    } else if (
      ts.isInterfaceDeclaration(node) ||
      ts.isTypeAliasDeclaration(node) ||
      ts.isVariableDeclaration(node) ||
      ts.isFunctionDeclaration(node) ||
      ts.isEnumDeclaration(node) ||
      ts.isModuleDeclaration(node) ||
      ts.isClassDeclaration(node)
    ) {
      if (!node.name || !ts.isIdentifier(node.name) || node.name.text.startsWith("__")) return;
      const symbol = checker.getSymbolAtLocation(node.name);
      if (symbol && !hidden(node)) web.set(symbol.name, symbol);
    }
  }
  for (const file of ["env.d.ts", "src/engine.d.ts", "src/indexeddb.d.ts"]) {
    const entry = program.getSourceFile(resolve(runtime, file));
    if (!entry) throw new Error(`${file} is missing from the app tsconfig.`);
    collect(entry);
  }

  function resolveSymbol(symbol: ts.Symbol) {
    return symbol.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol;
  }

  function typeName(exported: ts.Symbol) {
    const symbol = resolveSymbol(exported);
    const node = symbol.valueDeclaration;
    return node && checker.getTypeOfSymbolAtLocation(symbol, node).getSymbol()?.name;
  }

  function typeOnly(exported: ts.Symbol) {
    return (
      exported.declarations?.some(ts.isTypeOnlyExportDeclaration) ||
      !(resolveSymbol(exported).flags & ts.SymbolFlags.Value)
    );
  }

  const scopes = new WeakMap<ReadonlyMap<string, ts.Symbol>, Map<ts.Symbol, string>>();
  function exportNames(scope: ReadonlyMap<string, ts.Symbol>) {
    let names = scopes.get(scope);
    if (!names) {
      names = new Map([...scope].map(([name, symbol]) => [resolveSymbol(symbol), name]));
      scopes.set(scope, names);
    }
    return names;
  }
  function directTypes(exported: ts.Symbol) {
    const subject = resolveSymbol(exported);
    const found = new Set<ts.Symbol>();
    function visit(node: ts.Node) {
      if (ts.isBlock(node) || hidden(node)) return;
      const reference = ts.isTypeReferenceNode(node)
        ? node.typeName
        : ts.isExpressionWithTypeArguments(node)
          ? node.expression
          : undefined;
      if (reference) {
        const symbol = checker.getSymbolAtLocation(reference);
        if (symbol) found.add(resolveSymbol(symbol));
      }
      if (reference || !ts.isExpression(node)) ts.forEachChild(node, visit);
    }
    for (const d of subject.declarations ?? []) {
      visit(d);
      // An exported singleton's public members live on its interface, e.g. gamepads: Gamepads.
      if (ts.isVariableDeclaration(d)) {
        const shape = checker.getTypeOfSymbolAtLocation(subject, d).getSymbol();
        for (const decl of shape?.declarations ?? []) if (ts.isInterfaceDeclaration(decl)) visit(decl);
      }
    }
    found.delete(subject);
    return found;
  }
  function referencedTypes(scope: ReadonlyMap<string, ts.Symbol>, exported: ts.Symbol): string[] {
    const names = exportNames(scope);
    return [...directTypes(exported)].map((symbol) => names.get(symbol)).filter((name) => name !== undefined);
  }

  function memberSyntax(owner: string, member: ts.Symbol, at: ts.Node, docs = false): string[] {
    if (member.name.startsWith("#") || member.declarations?.every(hidden)) return [];
    let key = owner ? "." + member.name : member.name;
    if (member.name.startsWith("__@")) {
      const declaration = member.declarations
        ?.map(ts.getNameOfDeclaration)
        .find((d) => d && ts.isComputedPropertyName(d));
      if (!declaration) return [];
      key = declaration.getText();
    }
    const lines = signatures(member, owner + key, at);
    const comment = docs && ts.displayPartsToString(member.getDocumentationComment(checker));
    if (comment && lines.length) lines.unshift(`/** ${comment} */`);
    return lines;
  }

  function signatures(symbol: ts.Symbol, name: string, location: ts.Node): string[] {
    const type = checker.getTypeOfSymbolAtLocation(symbol, location);
    const optional = symbol.flags & ts.SymbolFlags.Optional ? "?" : "";
    const calls = checker.getNonNullableType(type).getCallSignatures();
    if (calls.length)
      return calls
        .filter((sig) => !hidden(sig.declaration))
        .map((sig) => `${name}${optional}${checker.signatureToString(sig, location, format)}`);
    const declarations = symbol.declarations ?? [];
    const readonly =
      (declarations.some(ts.isGetAccessorDeclaration) && !declarations.some(ts.isSetAccessorDeclaration)) ||
      declarations.some(
        (d) => ts.canHaveModifiers(d) && ts.getModifiers(d)?.some((m) => m.kind === ts.SyntaxKind.ReadonlyKeyword),
      );
    return [`${readonly ? "readonly " : ""}${name}${optional}: ${checker.typeToString(type, location, format)}`];
  }

  function classSyntax(symbol: ts.Symbol, name: string, node: ts.ClassDeclaration, docs: boolean): string {
    const generics = node.typeParameters?.map((p) => p.getText()).join(", ");
    const heritage = node.heritageClauses?.map((c) => " " + c.getText()).join("") ?? "";
    const header = `class ${name}${generics ? `<${generics}>` : ""}${heritage}`;
    const statics = checker.getTypeOfSymbolAtLocation(symbol, node);
    const constructors = statics.getConstructSignatures();
    const lines = constructors
      .filter((sig) => !hidden(sig.declaration))
      .map((sig) => {
        const ctor = checker.signatureToSignatureDeclaration(
          sig,
          ts.SyntaxKind.ConstructorType,
          node,
          ts.NodeBuilderFlags.NoTruncation,
        );
        if (!ctor) throw new Error(`Cannot describe the constructor for ${name}`);
        const params = ctor.parameters.map((p) => printer.printNode(ts.EmitHint.Unspecified, p, node.getSourceFile()));
        return `constructor(${params.join(", ")});`;
      });
    if (!lines.length && constructors.length) lines.push("private constructor();");
    for (const [owner, modifier] of [
      [statics, "static "],
      [checker.getDeclaredTypeOfSymbol(symbol), ""],
    ] as const) {
      for (const member of owner.getProperties()) {
        if (!member.declarations?.some((d) => d.parent === node)) continue;
        for (const line of memberSyntax("", member, node, docs)) {
          lines.push(line.startsWith("/**") ? line : modifier + line + ";");
        }
      }
    }
    return [header + " {", ...lines.map((line) => "  " + line), "}"].join("\n");
  }

  function describe(exported: ts.Symbol, docs = false): Declaration {
    const symbol = resolveSymbol(exported);
    const decls = symbol.declarations;
    if (!decls?.length) throw new Error(`Missing declaration for ${exported.name}`);
    const summary = ts.displayPartsToString(symbol.getDocumentationComment(checker));
    const lines: string[] = [];
    for (const d of decls) {
      if (hidden(d)) continue;
      if (ts.isInterfaceDeclaration(d) || ts.isTypeAliasDeclaration(d) || ts.isEnumDeclaration(d))
        lines.push(print(d, docs, summary));
      else if (ts.isModuleDeclaration(d)) {
        lines.push(
          `namespace ${exported.name} {`,
          ...checker
            .getExportsOfModule(symbol)
            .filter((child) => !resolveSymbol(child).declarations?.every(hidden))
            .map((child) => describe(child, docs).syntax)
            .filter(Boolean),
          "}",
        );
      } else if (ts.isClassDeclaration(d)) {
        lines.push(classSyntax(symbol, exported.name, d, docs));
      } else if (ts.isFunctionDeclaration(d) || ts.isVariableDeclaration(d)) {
        const type = checker.getTypeOfSymbolAtLocation(symbol, d);
        const constructors = type.getConstructSignatures().filter((sig) => !hidden(sig.declaration));
        const pairedInterface = Boolean(symbol.flags & ts.SymbolFlags.Interface) && type.getProperty("prototype");
        if (constructors.length) {
          lines.push(...constructors.map((sig) => `new ${exported.name}${checker.signatureToString(sig, d, format)}`));
        } else if (!pairedInterface) {
          lines.push(...signatures(symbol, exported.name, d));
        }
        if (
          pairedInterface ||
          constructors.length ||
          (docs && !d.getSourceFile().isDeclarationFile && type.flags & ts.TypeFlags.Object)
        ) {
          for (const prop of type.getProperties()) {
            if (
              prop.name !== "prototype" &&
              !prop.declarations?.every((decl) => program.isSourceFileDefaultLibrary(decl.getSourceFile()))
            )
              lines.push(...memberSyntax(exported.name, prop, d, docs));
          }
        }
      }
    }
    return {
      name: exported.name,
      source: source(decls[0]!),
      summary,
      syntax: [...new Set(lines)].join("\n"),
    };
  }

  // Resolve a card's exact public spelling. A grouped topic such as "Web Audio" keeps its authored examples.
  function symbolSyntax(exported: ts.Symbol, root: string, path: string[], expandObject: boolean): string | undefined {
    const symbol = resolveSymbol(exported);
    const d = symbol.valueDeclaration ?? symbol.declarations?.[0];
    if (!d) return undefined;
    const type = checker.getTypeOfSymbolAtLocation(symbol, d);
    const declared = checker.getDeclaredTypeOfSymbol(symbol);
    if (path.length) {
      let owners = [type, declared];
      let label = root;
      for (const [i, member] of path.entries()) {
        const owner = owners.find((candidate) => candidate.getProperty(member));
        const prop = owner?.getProperty(member);
        if (!owner || !prop) return undefined;
        label += `.${member}`;
        if (i === path.length - 1) return signatures(prop, label, d).join("\n");
        owners = [checker.getNonNullableType(checker.getTypeOfSymbolAtLocation(prop, d))];
      }
    }
    if (symbol.flags & ts.SymbolFlags.Interface && type.getProperty("prototype"))
      return describe(exported, expandObject).syntax;
    if (
      expandObject &&
      symbol.flags & ts.SymbolFlags.Value &&
      !(symbol.flags & ts.SymbolFlags.Class) &&
      type.flags & ts.TypeFlags.Object &&
      !type.getCallSignatures().length
    ) {
      const members = type.getProperties().filter((p) => p.name !== "prototype" && !p.name.startsWith("__@"));
      const lines = type
        .getConstructSignatures()
        .filter((sig) => !hidden(sig.declaration))
        .map((sig) => `new ${root}${checker.signatureToString(sig, d, format)}`);
      lines.push(...members.flatMap((p) => memberSyntax(root, p, d, true)));
      if (lines.length) return lines.join("\n");
    }
    return describe(exported, expandObject).syntax || undefined;
  }

  function syntax(scope: ReadonlyMap<string, ts.Symbol>, spelling: string, expandObject = false): string | undefined {
    const [root, ...path] = spelling.replace(/\(\)$/, "").split(".");
    const exported = root && scope.get(root);
    if (!exported) return undefined;
    const signature = symbolSyntax(exported, root, path, expandObject);
    if (!signature || scope === web) return signature;
    // Source-local types like BinaryReader's NumberArray cannot be imported or linked in the public index.
    const tokens = new Set(signature.match(/[\w$]+/g));
    const local: string[] = [];
    const sourceFile = resolveSymbol(exported).declarations?.[0]?.getSourceFile();
    for (const type of directTypes(exported)) {
      if (exportNames(scope).has(type) || !tokens.has(type.name)) continue;
      for (const decl of type.declarations ?? []) {
        if (
          decl.getSourceFile() === sourceFile &&
          (ts.isInterfaceDeclaration(decl) || ts.isTypeAliasDeclaration(decl)) &&
          !hidden(decl)
        ) {
          local.push(print(decl, false));
        }
      }
    }
    return [signature, ...local].join("\n\n");
  }

  return { web, std, describe, syntax, typeName, typeOnly, referencedTypes, moduleSummaries };
}

let cache: ReturnType<typeof loadBindings>;
export const bindings = () => (cache ??= loadBindings());

if (import.meta.main) {
  const { std, web, describe } = bindings();
  if (!web.size || !std.size) throw new Error("Empty Web or skathi:* reference.");
  for (const symbols of [web, ...std.values()]) {
    for (const symbol of symbols.values()) {
      if (!describe(symbol).syntax) throw new Error(`Empty declaration for ${symbol.name}.`);
    }
  }
  console.log(
    `${web.size} Web globals and types, ${[...std.values()].reduce((total, exports) => total + exports.size, 0)} exports across ${std.size} skathi:* modules.`,
  );
}
