import type ts from "typescript";

/** Preserve the observable spelling of RegExp.source and tagged template strings before escaping a bundle. */
export function preserveLiteralText(code: string): string {
  const t: typeof ts = require("typescript");
  const file = t.createSourceFile("bundle.js", code, t.ScriptTarget.Latest, true, t.ScriptKind.JS);
  const edits: { start: number; end: number; text: string }[] = [];
  const sites: string[] = [];
  const unicode = /[^\x00-\x7f]/;
  const names = new Set<string>();
  if (code.includes("\\u")) {
    function collectNames(node: ts.Node): void {
      if (t.isIdentifier(node)) names.add(node.text);
      t.forEachChild(node, collectNames);
    }
    collectNames(file);
  }
  let name = "__skLiteral";
  while (code.includes(name) || ["RegExp", "Site", "Sites"].some((suffix) => names.has(name + suffix))) name += "_";
  let usesRegExp = false;
  const quote = (s: string) =>
    JSON.stringify(s).replace(/[^\x00-\x7f]/g, (c) => "\\u" + c.charCodeAt(0).toString(16).padStart(4, "0"));
  const replace = (node: ts.Node, text: string) => edits.push({ start: node.getStart(file), end: node.end, text });
  // A literal is a MemberExpression. Preserve its precedence when its replacement is a call.
  function preserveNewTarget(node: ts.Node): void {
    let target: ts.Node = node;
    while (
      (t.isPropertyAccessExpression(target.parent) || t.isElementAccessExpression(target.parent)) &&
      target.parent.expression === target
    )
      target = target.parent;
    if (t.isNewExpression(target.parent) && target.parent.expression === target) {
      edits.push(
        { start: node.getStart(file), end: node.getStart(file), text: "(" },
        { start: node.end, end: node.end, text: ")" },
      );
    }
  }
  function visit(node: ts.Node): void {
    if (t.isRegularExpressionLiteral(node) && unicode.test(node.text)) {
      const end = node.text.lastIndexOf("/");
      replace(node, ` ${name}RegExp(${quote(node.text.slice(1, end))},${quote(node.text.slice(end + 1))})`);
      usesRegExp = true;
      preserveNewTarget(node);
    } else if (t.isTaggedTemplateExpression(node)) {
      const template = node.template;
      const parts = t.isTemplateExpression(template)
        ? [template.head, ...template.templateSpans.map((s) => s.literal)]
        : [template];
      const raw = parts.map((part) => (part.rawText ?? part.text).replace(/\r\n?/g, "\n"));
      if (raw.some((value) => unicode.test(value))) {
        const cooked = raw.map((value) => {
          let valid = true;
          const scanner = t.createScanner(
            t.ScriptTarget.Latest,
            false,
            t.LanguageVariant.Standard,
            "`" + value + "`",
            () => {
              valid = false;
            },
          );
          scanner.scan();
          scanner.reScanTemplateHeadOrNoSubstitutionTemplate();
          return valid ? quote(scanner.getTokenValue()) : "void 0";
        });
        const index = sites.push(`${name}Site([${cooked.join(",")}],[${raw.map(quote).join(",")}])`) - 1;
        const args = `(${name}Sites[${index}]`;
        if (t.isTemplateExpression(template)) {
          replace(template.head, args + ",(");
          for (const span of template.templateSpans)
            replace(span.literal, t.isTemplateTail(span.literal) ? "))" : "),(");
        } else {
          replace(template, args + ")");
        }
        preserveNewTarget(node);
      }
    }
    t.forEachChild(node, visit);
  }
  visit(file);
  if (!edits.length) return code;
  let prologueEnd = code.startsWith("#!") ? code.indexOf("\n") + 1 : 0;
  for (const statement of file.statements) {
    if (!t.isExpressionStatement(statement) || !t.isStringLiteral(statement.expression)) break;
    prologueEnd = statement.end;
  }
  const header = [
    usesRegExp && `var ${name}RegExp=/(?:)/.constructor;`,
    sites.length &&
      `function ${name}Site(c,r){var O=({}).constructor;return O.freeze(O.defineProperty(c,"raw",{value:O.freeze(r)}))}var ${name}Sites=[${sites.join(",")}];`,
  ]
    .filter(Boolean)
    .join("\n");
  edits.unshift({ start: prologueEnd, end: prologueEnd, text: ";\n" + header + "\n" });
  const output: string[] = [];
  let from = 0;
  for (const edit of edits.sort((a, b) => a.start - b.start || a.end - b.end)) {
    output.push(code.slice(from, edit.start), edit.text);
    from = edit.end;
  }
  output.push(code.slice(from));
  return output.join("");
}
