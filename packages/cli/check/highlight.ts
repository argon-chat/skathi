import type ts from "typescript";
import type { Paint } from "../cli/term";
import type { TypeScript } from "./program";

export type Highlight = (code: string) => string;

/** Colours one line of TS/JS with the TypeScript scanner; a line cut mid-token still prints as is. */
export function highlighter(typescript: TypeScript, p: Paint): Highlight {
  const k = typescript.SyntaxKind;
  const style = (kind: ts.SyntaxKind): ((text: string) => string) | null => {
    if (kind === k.SingleLineCommentTrivia || kind === k.MultiLineCommentTrivia) return p.dim;
    if (
      kind === k.StringLiteral ||
      kind === k.NoSubstitutionTemplateLiteral ||
      kind === k.TemplateHead ||
      kind === k.TemplateMiddle ||
      kind === k.TemplateTail
    )
      return p.green;
    if (kind === k.NumericLiteral || kind === k.BigIntLiteral) return p.cyan;
    if (kind >= k.FirstKeyword && kind <= k.LastKeyword) return p.magenta;
    return null;
  };
  return (code) => {
    if (!p.on) return code;
    const scanner = typescript.createScanner(
      typescript.ScriptTarget.Latest,
      false,
      typescript.LanguageVariant.Standard,
      code,
    );
    let out = "";
    for (let kind = scanner.scan(); kind !== k.EndOfFileToken; kind = scanner.scan()) {
      const text = scanner.getTokenText();
      const paint = style(kind);
      out += paint ? paint(text) : text;
    }
    return out;
  };
}
