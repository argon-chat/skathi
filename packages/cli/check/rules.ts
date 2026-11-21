import type ts from "typescript";
import { Builtins, LOCKED, LOCKED_OBJECTS, OBJECTS, REPLACEABLE, memberPath } from "./builtins";
import { rangeOf, type Finding } from "./finding";
import type { TypeScript, Unit } from "./program";

const DEFINE_CALLS = new Set([
  "Object.defineProperty",
  "Object.defineProperties",
  "Object.assign",
  "Reflect.set",
  "Reflect.defineProperty",
  "Reflect.deleteProperty",
]);
const PROTOTYPE_CALLS = new Set(["Object.setPrototypeOf", "Reflect.setPrototypeOf"]);
const TIMERS = new Set(["setTimeout", "setInterval", "setImmediate"]);

/** What changing `path` on its holder does under AOT: null when it is allowed. */
function change(holder: string, path: string): "locked" | "locked-object" | null {
  // The engine's table leaves the host's `console` methods open; a Skathi image does not.
  if (LOCKED.has(path) || holder === "console") return "locked";
  if (LOCKED_OBJECTS.has(holder) && !REPLACEABLE.has(path)) return "locked-object";
  return null;
}

export class Rules {
  constructor(
    private readonly typescript: TypeScript,
    private readonly checker: ts.TypeChecker,
    private readonly builtins: Builtins,
  ) {}

  run(unit: Unit, file: ts.SourceFile, out: Finding[]): void {
    const t = this.typescript;
    const report = (code: string, message: string, start: number, end: number) =>
      out.push({ code, message, file: unit.path, range: rangeOf(file, start, end), related: [] });
    const at = (node: ts.Node) => [node.getStart(file), node.getEnd()] as const;

    const visit = (node: ts.Node): void => {
      if (t.isIterationStatement(node, false) && this.tight(node.statement)) this.tightLoop(node, file, report);
      if (t.isBinaryExpression(node) && this.isAssignment(node.operatorToken.kind)) {
        this.assignment(node, (code, message) => report(code, message, ...at(node.left)));
      } else if (t.isDeleteExpression(node)) {
        this.deletion(node, (code, message) => report(code, message, ...at(node.expression)));
      } else if (t.isCallExpression(node) || t.isNewExpression(node)) {
        this.call(node, file, report);
      } else if (t.isIdentifier(node) && node.text === "eval") {
        this.evalReference(node, (message) => report("SK2001", message, ...at(node)));
      } else if (t.isPropertyAccessExpression(node) && node.name.text === "eval") {
        if (!t.isTypeOfExpression(node.parent) && this.builtins.path(node.expression) === "globalThis")
          report("SK2001", "`eval` (through the global object) runs interpreted under AOT, without a JIT", ...at(node));
        return visit(node.expression);
      }
      t.forEachChild(node, visit);
    };
    visit(file);
  }

  /** A loop body that is all arithmetic and element access: no calls but `Math`'s, no closures. */
  private tight(body: ts.Node): boolean {
    const t = this.typescript;
    const plain = (node: ts.Node): boolean => {
      if (t.isFunctionLike(node) || t.isAwaitExpression(node) || t.isYieldExpression(node)) return false;
      if (t.isNewExpression(node) || t.isTaggedTemplateExpression(node)) return false;
      if (t.isCallExpression(node) && !(this.builtins.path(node.expression) ?? "").startsWith("Math.")) return false;
      return t.forEachChild(node, (child) => !plain(child)) !== true;
    };
    return plain(body);
  }

  private tightLoop(
    loop: ts.IterationStatement,
    file: ts.SourceFile,
    report: (code: string, message: string, start: number, end: number) => void,
  ): void {
    const t = this.typescript;
    const k = t.SyntaxKind;
    const condition = t.isForStatement(loop)
      ? loop.condition
      : t.isWhileStatement(loop) || t.isDoStatement(loop)
        ? loop.expression
        : undefined;
    if (
      condition &&
      t.isBinaryExpression(condition) &&
      condition.operatorToken.kind >= k.LessThanToken &&
      condition.operatorToken.kind <= k.GreaterThanEqualsToken
    ) {
      for (const side of [condition.left, condition.right]) {
        if (t.isIdentifier(side) && this.numberParameter(side))
          report(
            "SK2005",
            `\`${side.text}\` bounds a tight loop as a \`number\`; an AOT image cannot assume it is an integer, and the loop runs about three times slower than with an integer bound`,
            side.getStart(file),
            side.getEnd(),
          );
      }
    }
    const stores = (node: ts.Node): void => {
      if (node !== loop.statement && t.isIterationStatement(node, false)) return;
      if (
        t.isBinaryExpression(node) &&
        this.isAssignment(node.operatorToken.kind) &&
        t.isElementAccessExpression(node.left) &&
        !t.isLiteralExpression(node.left.argumentExpression) &&
        this.numberArray(node.left.expression)
      )
        report(
          "SK2006",
          `\`${node.left.expression.getText(file)}\` is a \`number[]\` written by index in a tight loop; an AOT image stores into a typed array about twice as fast`,
          node.left.getStart(file),
          node.left.getEnd(),
        );
      t.forEachChild(node, stores);
    };
    stores(loop.statement);
  }

  private numberParameter(name: ts.Identifier): boolean {
    const t = this.typescript;
    const declaration = this.checker.getSymbolAtLocation(name)?.valueDeclaration;
    if (!declaration || !t.isParameter(declaration) || declaration.dotDotDotToken) return false;
    return this.checker.getTypeAtLocation(name).flags === t.TypeFlags.Number;
  }

  private numberArray(target: ts.Expression): boolean {
    const type = this.checker.getTypeAtLocation(target);
    if (type.symbol?.name !== "Array") return false;
    const [element] = this.checker.getTypeArguments(type as ts.TypeReference);
    return element?.flags === this.typescript.TypeFlags.Number;
  }

  private isAssignment(kind: ts.SyntaxKind): boolean {
    const k = this.typescript.SyntaxKind;
    return kind >= k.FirstAssignment && kind <= k.LastAssignment;
  }

  /** The members `node` can name: one, or each string of a key whose type is a union of literals. */
  private targets(node: ts.Expression): { holder: string; name: string; path: string }[] {
    const t = this.typescript;
    if (!t.isPropertyAccessExpression(node) && !t.isElementAccessExpression(node)) return [];
    const key = t.isPropertyAccessExpression(node) ? node.name : node.argumentExpression;
    // In `holder[key]` an identifier is a variable, not the member's name.
    const name = t.isElementAccessExpression(node) && t.isIdentifier(key) ? null : this.builtins.memberName(key);
    const names = name !== null ? [name] : this.keys(key);
    if (!names.length) return [];
    const holder = this.builtins.path(node.expression);
    if (holder === null) return [];
    return names.map((name) => ({ holder, name, path: memberPath(holder, name) }));
  }

  private keys(key: ts.Node): string[] {
    try {
      const type = this.checker.getTypeAtLocation(key);
      const parts = type.isUnion() ? type.types : [type];
      return parts.every((part) => part.isStringLiteral())
        ? parts.map((part) => (part as ts.StringLiteralType).value)
        : [];
    } catch {
      return [];
    }
  }

  private assignment(node: ts.BinaryExpression, report: (code: string, message: string) => void): void {
    const t = this.typescript;
    const left = node.left;
    if (t.isIdentifier(left)) {
      if (
        LOCKED.has(left.text) &&
        !left.text.includes(".") &&
        this.builtins.isGlobal(left) &&
        !this.guarded(node, left.text, left.text)
      )
        report("SK1004", `the global \`${left.text}\` is assigned; AOT code cannot replace the language's globals`);
      return;
    }
    for (const target of this.targets(left)) this.assigned(node, target, report);
  }

  private assigned(
    node: ts.BinaryExpression,
    target: { holder: string; name: string; path: string },
    report: (code: string, message: string) => void,
  ): void {
    const t = this.typescript;
    if (target.name === "__proto__") {
      if (OBJECTS.has(target.holder))
        report(
          "SK1003",
          `\`${target.holder}.__proto__\` is assigned; AOT code cannot change what a built-in inherits from`,
        );
      return;
    }
    const kind = change(target.holder, target.path);
    // `??=` and `||=` write only when the member is missing, which a built-in never is.
    const k = t.SyntaxKind;
    const fills =
      node.operatorToken.kind === k.QuestionQuestionEqualsToken || node.operatorToken.kind === k.BarBarEqualsToken;
    if (
      !kind ||
      (fills && kind === "locked") ||
      this.restores(node.right, target.path) ||
      this.guarded(node, target.path, target.name)
    )
      return;
    if (target.holder === "globalThis")
      report("SK1004", `\`globalThis.${target.name}\` is replaced; AOT code cannot replace the language's globals`);
    else if (kind === "locked-object")
      report("SK1001", `\`${target.path}\` is added; ${target.holder} takes no new members under AOT`);
    else report("SK1001", `\`${target.path}\` is replaced; AOT code cannot change built-in methods`);
  }

  private deletion(node: ts.DeleteExpression, report: (code: string, message: string) => void): void {
    for (const target of this.targets(node.expression)) {
      if (change(target.holder, target.path) !== "locked" || this.guarded(node, target.path, target.name)) continue;
      if (target.holder === "globalThis")
        report("SK1004", `\`globalThis.${target.name}\` is deleted; AOT code cannot remove the language's globals`);
      else report("SK1001", `\`${target.path}\` is deleted; AOT code cannot change built-in methods`);
    }
  }

  private call(
    node: ts.CallExpression | ts.NewExpression,
    file: ts.SourceFile,
    report: (code: string, message: string, start: number, end: number) => void,
  ): void {
    const t = this.typescript;
    const args: readonly ts.Expression[] = node.arguments ?? [];
    if (t.isCallExpression(node) && node.expression.kind === t.SyntaxKind.ImportKeyword) {
      const spec = args[0];
      if (spec && !t.isStringLiteral(spec) && !t.isNoSubstitutionTemplateLiteral(spec))
        report(
          "SK2004",
          "`import()` with a computed specifier; AOT loads only the modules the build could find",
          spec.getStart(file),
          spec.getEnd(),
        );
      return;
    }
    const callee = this.builtins.path(node.expression);
    if (callee === null) return;
    const span = (n: ts.Node) => [n.getStart(file), n.getEnd()] as const;
    if (callee === "Function") {
      const what = t.isNewExpression(node) ? "`new Function(…)`" : "`Function(…)`";
      report(
        "SK2002",
        `${what} compiles a string at run time; under AOT it runs interpreted, without a JIT`,
        node.getStart(file),
        node.expression.getEnd(),
      );
      return;
    }
    if (TIMERS.has(callee)) {
      const code = args[0];
      if (code && this.isString(code))
        report(
          "SK2003",
          `\`${callee}\` is given a string of code; Skathi timers take functions only, so it never runs`,
          ...span(code),
        );
      return;
    }
    if (PROTOTYPE_CALLS.has(callee)) {
      const target = args[0] && this.builtins.path(args[0]);
      if (target && (OBJECTS.has(target) || target === "globalThis") && !this.guarded(node, target, ""))
        report(
          "SK1003",
          `\`${callee}\` changes what \`${target}\` inherits from; AOT built-ins cannot be rewired`,
          ...span(node),
        );
      return;
    }
    if (!DEFINE_CALLS.has(callee)) return;
    const holder = args[0] && this.builtins.path(args[0]);
    if (!holder || !(OBJECTS.has(holder) || holder === "globalThis")) return;
    const members: { name: string; node: ts.Node }[] = [];
    const fromObject = (arg: ts.Expression | undefined) => {
      if (!arg || !t.isObjectLiteralExpression(arg)) return;
      for (const property of arg.properties) {
        if (!property.name) continue;
        const name = this.builtins.memberName(property.name);
        if (name !== null) members.push({ name, node: property.name });
      }
    };
    if (callee === "Object.defineProperties") fromObject(args[1]);
    else if (callee === "Object.assign") for (const arg of args.slice(1)) fromObject(arg);
    else if (args[1]) {
      const name = this.builtins.memberName(args[1]);
      if (name !== null) members.push({ name, node: args[1] });
    }
    for (const member of members) {
      const path = memberPath(holder, member.name);
      const kind = change(holder, path);
      if (!kind || this.guarded(node, path, member.name)) continue;
      const shown = holder === "globalThis" ? `globalThis.${member.name}` : path;
      const message = callee.startsWith("Reflect.")
        ? `\`${callee}\` cannot change \`${shown}\` under AOT: it returns false and leaves the built-in as it was`
        : kind === "locked-object"
          ? `\`${callee}\` adds \`${shown}\`; ${holder} takes no new members under AOT`
          : `\`${callee}\` redefines \`${shown}\`; built-ins are read-only under AOT`;
      report("SK1002", message, ...span(member.node));
    }
  }

  private evalReference(node: ts.Identifier, report: (message: string) => void): void {
    const t = this.typescript;
    const parent = node.parent;
    if (t.isTypeOfExpression(parent)) return;
    if (t.isPropertyAccessExpression(parent) && parent.name === node) return;
    if (t.isQualifiedName(parent) && parent.right === node) return;
    if (t.isPropertyAssignment(parent) && parent.name === node) return;
    if (t.isMethodDeclaration(parent) || t.isPropertyDeclaration(parent) || t.isPropertySignature(parent)) return;
    if (t.isBindingElement(parent) || t.isVariableDeclaration(parent) || t.isParameter(parent)) return;
    if (t.isImportSpecifier(parent) || t.isExportSpecifier(parent)) return;
    if (!this.builtins.isGlobal(node)) return;
    const direct = t.isCallExpression(parent) && parent.expression === node;
    report(
      direct
        ? "`eval` runs interpreted under AOT, without a JIT"
        : "a reference to `eval` (an indirect eval) runs interpreted under AOT, without a JIT",
    );
  }

  private isString(node: ts.Expression): boolean {
    const t = this.typescript;
    if (t.isStringLiteralLike(node) || t.isTemplateExpression(node)) return true;
    try {
      const type = this.checker.getTypeAtLocation(node);
      return (type.flags & t.TypeFlags.StringLike) !== 0;
    } catch {
      return false;
    }
  }

  /** `x = x` or `x = saved` where `saved` was read from `x`: putting the original back is allowed. */
  private restores(value: ts.Expression, path: string): boolean {
    const t = this.typescript;
    let node = value;
    while (t.isParenthesizedExpression(node)) node = node.expression;
    if (
      t.isBinaryExpression(node) &&
      (node.operatorToken.kind === t.SyntaxKind.BarBarToken ||
        node.operatorToken.kind === t.SyntaxKind.QuestionQuestionToken)
    )
      return this.restores(node.left, path);
    if (this.builtins.path(node) === path) return true;
    if (t.isIdentifier(node)) {
      const declaration = this.checker.getSymbolAtLocation(node)?.valueDeclaration;
      return (
        !!declaration &&
        t.isVariableDeclaration(declaration) &&
        !!declaration.initializer &&
        this.builtins.path(declaration.initializer) === path
      );
    }
    return false;
  }

  /** A polyfill guard around the change: `if (!X.prototype.m)`, `"m" in X.prototype`, `X.m || (X.m = …)`. */
  private guarded(node: ts.Node, path: string, name: string): boolean {
    const t = this.typescript;
    const mentions = (condition: ts.Node): boolean => {
      let found = false;
      const look = (n: ts.Node): void => {
        if (found) return;
        if (
          (t.isPropertyAccessExpression(n) || t.isElementAccessExpression(n) || t.isIdentifier(n)) &&
          this.builtins.path(n) === path
        )
          found = true;
        else if (name && t.isStringLiteralLike(n) && n.text === name) found = true;
        // `if (!Runtime.isAot())`: the change does not run from an image.
        else if (t.isPropertyAccessExpression(n) && n.name.text === "isAot") found = true;
        else t.forEachChild(n, look);
      };
      look(condition);
      return found;
    };
    let child: ts.Node = node;
    for (let parent = node.parent; parent; child = parent, parent = parent.parent) {
      if (t.isFunctionLike(parent) || t.isClassLike(parent)) return false;
      if (t.isIfStatement(parent) && child !== parent.expression && mentions(parent.expression)) return true;
      if (t.isConditionalExpression(parent) && child !== parent.condition && mentions(parent.condition)) return true;
      if (
        t.isBinaryExpression(parent) &&
        child === parent.right &&
        [t.SyntaxKind.AmpersandAmpersandToken, t.SyntaxKind.BarBarToken, t.SyntaxKind.QuestionQuestionToken].includes(
          parent.operatorToken.kind,
        ) &&
        mentions(parent.left)
      )
        return true;
      if (t.isBlock(parent) || t.isSourceFile(parent) || t.isModuleBlock(parent) || t.isCaseClause(parent)) {
        // An early exit before the change: `if (Array.prototype.at) return;`
        for (const statement of parent.statements) {
          if (statement === child) break;
          if (t.isIfStatement(statement) && mentions(statement.expression) && this.exits(statement.thenStatement))
            return true;
        }
      }
    }
    return false;
  }

  private exits(statement: ts.Statement): boolean {
    const t = this.typescript;
    if (t.isReturnStatement(statement) || t.isThrowStatement(statement)) return true;
    return t.isBlock(statement) && statement.statements.some((s) => t.isReturnStatement(s) || t.isThrowStatement(s));
  }
}
