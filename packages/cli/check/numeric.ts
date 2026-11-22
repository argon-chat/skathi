import type ts from "typescript";
import { brandOf } from "../numeric";
import { rangeOf, type Finding } from "./finding";
import type { TypeScript, Unit } from "./program";

/** SK7001: two `skathi:numeric` types meeting in one operator, which Rust would refuse without a conversion. */
export class NumericRules {
  private readonly operators: ReadonlySet<ts.SyntaxKind>;

  constructor(
    private readonly typescript: TypeScript,
    private readonly checker: ts.TypeChecker,
    private readonly numericFile: string,
  ) {
    const k = typescript.SyntaxKind;
    this.operators = new Set([
      k.PlusToken,
      k.MinusToken,
      k.AsteriskToken,
      k.SlashToken,
      k.PercentToken,
      k.AsteriskAsteriskToken,
      k.AmpersandToken,
      k.BarToken,
      k.CaretToken,
      k.LessThanLessThanToken,
      k.GreaterThanGreaterThanToken,
      k.GreaterThanGreaterThanGreaterThanToken,
      k.LessThanToken,
      k.GreaterThanToken,
      k.LessThanEqualsToken,
      k.GreaterThanEqualsToken,
      k.PlusEqualsToken,
      k.MinusEqualsToken,
      k.AsteriskEqualsToken,
      k.SlashEqualsToken,
      k.PercentEqualsToken,
      k.AsteriskAsteriskEqualsToken,
      k.AmpersandEqualsToken,
      k.BarEqualsToken,
      k.CaretEqualsToken,
      k.LessThanLessThanEqualsToken,
      k.GreaterThanGreaterThanEqualsToken,
      k.GreaterThanGreaterThanGreaterThanEqualsToken,
    ]);
  }

  run(unit: Unit, file: ts.SourceFile, out: Finding[]): void {
    const t = this.typescript;
    const brand = (node: ts.Expression) => {
      if (t.isLiteralExpression(node) || t.isPrefixUnaryExpression(node)) return null;
      try {
        return brandOf(this.checker, this.checker.getTypeAtLocation(node), this.numericFile);
      } catch {
        return null;
      }
    };
    const visit = (node: ts.Node): void => {
      if (t.isBinaryExpression(node) && this.operators.has(node.operatorToken.kind)) {
        const left = brand(node.left);
        const right = left && brand(node.right);
        if (left && right && left !== right) {
          const operator = node.operatorToken.getText(file);
          out.push({
            code: "SK7001",
            message: `\`${operator}\` combines a \`${left}\` and a \`${right}\` without a conversion, so the result is neither type`,
            file: unit.path,
            range: rangeOf(file, node.getStart(file), node.getEnd()),
            help: `Convert one side to the other's type first: \`${right}(${node.left.getText(file)})\`, or \`${left}(${node.right.getText(file)})\`.`,
            related: [],
          });
        }
      }
      t.forEachChild(node, visit);
    };
    visit(file);
  }
}
