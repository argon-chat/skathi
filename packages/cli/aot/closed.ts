import ts from "typescript";
import { Builtins } from "../check/builtins";
import type { nativeInput } from "./transform";
import { INPUT_MODULE, type FeatureUse, type Uses } from "./uses";

const NO_MODULE = 0xffffffff;
const constructors = new Set(["%AsyncFunction%", "%AsyncGeneratorFunction%", "%GeneratorFunction%"]);
// These are members of Bun's host namespace, which the Skathi realm does not install.
const bunServices = new Set([
  "Bun.build",
  "Bun.serve",
  "Bun.Transpiler",
  "Bun.Transpiler.prototype",
  "Bun.registerMacro",
]);
const equality = new Set([
  ts.SyntaxKind.EqualsEqualsToken,
  ts.SyntaxKind.EqualsEqualsEqualsToken,
  ts.SyntaxKind.ExclamationEqualsToken,
  ts.SyntaxKind.ExclamationEqualsEqualsToken,
]);

export const CLOSED_POLICY = "exclude=LibraryText,EvaluatedText,HostEvaluatedText,CodeOutsideImage";

function anonymousHostUse(use: FeatureUse): boolean {
  return use.feature === "Interpreter" && use.kind === "Unnamed" && bunServices.has(use.builtin ?? "");
}

// Opaque host globals have no JSC layout. A computed lookup can hand out eval or Function
// even when neither intrinsic has a named use in the engine report.
function hostCallTargets(file: ts.SourceFile, builtins: Builtins, checker: ts.TypeChecker) {
  const values = new Map<ts.Symbol, ts.Expression[]>();
  function remember(id: ts.Node, expression: ts.Expression) {
    if (!ts.isIdentifier(id)) return;
    const symbol = checker.getSymbolAtLocation(id);
    if (symbol) {
      const prior = values.get(symbol);
      if (prior) prior.push(expression);
      else values.set(symbol, [expression]);
    }
  }
  function collect(node: ts.Node): void {
    if (ts.isVariableDeclaration(node) && node.initializer) remember(node.name, node.initializer);
    if (
      ts.isBinaryExpression(node) &&
      node.operatorToken.kind >= ts.SyntaxKind.FirstAssignment &&
      node.operatorToken.kind <= ts.SyntaxKind.LastAssignment
    )
      remember(node.left, node.right);
    ts.forEachChild(node, collect);
  }
  collect(file);
  function mayNameEvaluator(key: ts.Expression, seen = new Set<ts.Symbol>()): boolean {
    if (ts.isStringLiteralLike(key)) return key.text === "eval" || key.text === "Function";
    if (ts.isNumericLiteral(key)) return false;
    if (!ts.isIdentifier(key)) return true;
    const symbol = checker.getSymbolAtLocation(key);
    const sources = symbol && values.get(symbol);
    if (!symbol || !sources || seen.has(symbol)) return true;
    seen.add(symbol);
    const possible = sources.some((source) => mayNameEvaluator(source, seen));
    seen.delete(symbol);
    return possible;
  }
  function matches(node: ts.Expression, global: boolean, seen: Set<ts.Symbol>): boolean {
    if (ts.isParenthesizedExpression(node)) return matches(node.expression, global, seen);
    if (ts.isConditionalExpression(node))
      return matches(node.whenTrue, global, seen) || matches(node.whenFalse, global, seen);
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.CommaToken)
      return matches(node.right, global, seen);
    if (ts.isIdentifier(node)) {
      if (global && builtins.path(node) === "globalThis") return true;
      const symbol = checker.getSymbolAtLocation(node);
      if (!symbol || seen.has(symbol)) return false;
      seen.add(symbol);
      const found = values.get(symbol)?.some((rhs) => matches(rhs, global, seen)) ?? false;
      seen.delete(symbol);
      return found;
    }
    if (global) return builtins.path(node) === "globalThis";
    if (ts.isElementAccessExpression(node))
      return matches(node.expression, true, seen) && mayNameEvaluator(node.argumentExpression);
    if (ts.isPropertyAccessExpression(node) && ["call", "apply", "bind"].includes(node.name.text))
      return matches(node.expression, false, seen);
    if (ts.isCallExpression(node) && builtins.path(node.expression) === "Reflect.get") {
      const [holder, key] = node.arguments;
      return !!holder && !!key && matches(holder, true, seen) && mayNameEvaluator(key);
    }
    return false;
  }
  return {
    byName: (callee: ts.Expression) => matches(callee, false, new Set()),
    onlyInitializer: (name: ts.Identifier) => {
      const symbol = checker.getSymbolAtLocation(name);
      return !!symbol && values.get(symbol)?.length === 1;
    },
  };
}

/** Check the actual bundle too: the flow engine keeps only one witness per intrinsic and cause. */
function inspectInput(text: string, byteOffset: number, candidates: Set<number>) {
  const file = ts.createSourceFile("native-input.js", text, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const options = { allowJs: true, noLib: true, noResolve: true };
  const host = ts.createCompilerHost(options);
  host.getSourceFile = (name) => (name === file.fileName ? file : undefined);
  const program = ts.createProgram([file.fileName], options, host);
  const checker = program.getTypeChecker();
  const builtins = new Builtins(ts, program, checker);
  const targets = hostCallTargets(file, builtins, checker);
  const bytes = Buffer.from(text);
  const starts = new Map(
    [...candidates]
      .filter((p) => p >= byteOffset && p < byteOffset + bytes.length)
      .map((p) => [bytes.toString("utf8", 0, p - byteOffset).length, p]),
  );
  const references = new Set<number>();
  const uses: Uses["unseen"] = [];
  const flag = (node: ts.Node, cause: string, builtin: string | null) => {
    uses.push({
      cause,
      builtin,
      module: INPUT_MODULE,
      position: byteOffset + Buffer.byteLength(text.slice(0, node.getStart(file))),
    });
  };
  const visit = (node: ts.Node): void => {
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.initializer &&
      ts.isPropertyAccessExpression(node.initializer)
    ) {
      const offset = starts.get(node.getStart(file));
      if (offset !== undefined) {
        const path = builtins.path(node.initializer);
        // JSC can report Function at the binding of this locked, non-callback intrinsic.
        if (path === "Array.isArray" && targets.onlyInitializer(node.name)) references.add(offset);
      }
    }
    if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
      const callee = node.expression;
      const path = builtins.path(callee);
      const target = path === "Reflect.apply" || path === "Reflect.construct" ? node.arguments?.[0] : callee;
      if (target && targets.byName(target)) flag(target, "RunsUnseenCode", "computed host function");
      if (
        callee.kind === ts.SyntaxKind.ImportKeyword &&
        node.arguments?.[0] &&
        !ts.isStringLiteralLike(node.arguments[0])
      )
        flag(callee, "LoadsModuleByName", null);
    }
    let evaluator: string | null = null;
    if (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)) {
      const path = builtins.path(node);
      if (path === "eval" || path === "Function") evaluator = path;
      const member = ts.isPropertyAccessExpression(node)
        ? node.name.text
        : ts.isStringLiteralLike(node.argumentExpression)
          ? node.argumentExpression.text
          : null;
      if (member === "constructor") {
        let holder: ts.Expression = node.expression;
        while (ts.isParenthesizedExpression(holder)) holder = holder.expression;
        if (
          ts.isArrowFunction(holder) ||
          ts.isFunctionExpression(holder) ||
          builtins.path(holder) === "Function.prototype"
        )
          evaluator = "Function constructor";
      }
    } else if (
      ts.isIdentifier(node) &&
      (node.text === "eval" || node.text === "Function") &&
      !(ts.isPropertyAccessExpression(node.parent) && node.parent.name === node) &&
      !(ts.isPropertyAssignment(node.parent) && node.parent.name === node) &&
      !(ts.isMethodDeclaration(node.parent) && node.parent.name === node) &&
      builtins.isGlobal(node)
    ) {
      evaluator = node.text;
    }
    if (evaluator) {
      let value = node;
      while (ts.isParenthesizedExpression(value.parent)) value = value.parent;
      const next = value.parent;
      const inspected =
        ts.isTypeOfExpression(next) ||
        (ts.isBinaryExpression(next) && equality.has(next.operatorToken.kind)) ||
        (ts.isPropertyAccessExpression(next) && ["name", "prototype", "length"].includes(next.name.text));
      if (!inspected) flag(value, "EvaluatesText", evaluator);
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return { references, uses };
}

/** Reconcile a closed image's report with the same input analyzed with string evaluation enabled. */
export function closedUses(closed: Uses, open: Uses, input: ReturnType<typeof nativeInput>): Uses {
  const candidates = new Set(
    [...open.features, ...open.unseen]
      .filter((r) => r.module === INPUT_MODULE && r.builtin === "Function")
      .map((r) => r.position),
  );
  const inspected = input.sections.map((s) => inspectInput(s.source, s.byteOffset, candidates));
  const references = new Set(inspected.flatMap((s) => [...s.references]));
  const uses = inspected.flatMap((s) => s.uses);
  const readOnlyFunction = (r: { module: number; position: number; builtin: string | null }) =>
    r.module === INPUT_MODULE && r.builtin === "Function" && references.has(r.position);
  const syntheticConstructor = (r: { module: number; builtin: string | null }) =>
    r.module === NO_MODULE && constructors.has(r.builtin ?? "");
  const openFeatures = open.features.filter((r) => {
    if (r.feature !== "Interpreter") return true;
    if (anonymousHostUse(r)) return false;
    if (r.kind === "Named" && r.module === NO_MODULE && r.builtin === "Bun.build") return false;
    if (r.kind === "Unnamed" && (syntheticConstructor(r) || (r.module === NO_MODULE && r.builtin === "eval")))
      return false;
    return r.kind !== "Named" || !readOnlyFunction(r);
  });
  const inferredEvaluation = (r: Uses["unseen"][number]) =>
    r.cause === "EvaluatesText" && (syntheticConstructor(r) || readOnlyFunction(r));
  const mayImportFromInferredText = open.unseen.some(inferredEvaluation);
  const openUnseen = open.unseen.filter((r) => {
    if (inferredEvaluation(r)) return false;
    return !(
      mayImportFromInferredText &&
      r.cause === "LoadsModuleByName" &&
      r.module === NO_MODULE &&
      r.builtin === null
    );
  });
  return {
    ...closed,
    features: [...closed.features.filter((r) => !anonymousHostUse(r)), ...openFeatures],
    unseen: [...closed.unseen, ...openUnseen, ...uses],
  };
}
