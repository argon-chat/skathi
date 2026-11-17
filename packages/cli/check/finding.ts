import { relative, sep } from "node:path";
import type ts from "typescript";
import { codeInfo } from "./codes";
import { Suppressions } from "./config";
import type { Packages } from "./packages";
import type { Diagnostic, Range } from "./schema";

export interface FindingRelated {
  /** Absolute. */
  file: string;
  range: Range;
  message: string;
}

/** A diagnostic before it is made relative, attributed and filtered. */
export interface Finding {
  code: string;
  message: string;
  /** Absolute. */
  file: string;
  range: Range;
  help?: string | undefined;
  related: FindingRelated[];
}

export const ZERO_RANGE: Range = { start: { line: 0, character: 0 }, end: { line: 0, character: 0 } };

export function rangeOf(file: ts.SourceFile, start: number, end: number): Range {
  const a = file.getLineAndCharacterOfPosition(start);
  const b = file.getLineAndCharacterOfPosition(end);
  return { start: { line: a.line, character: a.character }, end: { line: b.line, character: b.character } };
}

export const nodeRange = (file: ts.SourceFile, node: ts.Node) => rangeOf(file, node.getStart(file), node.getEnd());

/** Packages the SDK provides and answers for under AOT; an app cannot swap or retype them. */
export const PLATFORM = new Set([
  "vue",
  "@vue/runtime-core",
  "@vue/runtime-dom",
  "@vue/reactivity",
  "@vue/shared",
  "@skathi-native/runtime",
]);

export const posix = (path: string) => path.split(sep).join("/");

/** Findings as sorted diagnostics relative to `root`: the platform's own dropped, `check.allow` applied. */
export function diagnosticsOf(
  findings: Finding[],
  root: string,
  packages: Packages,
  allow: Map<string, string[]>,
): { diagnostics: Diagnostic[]; suppressed: number } {
  const rel = (file: string) => posix(relative(root, file));
  const suppress = new Suppressions(allow);
  const diagnostics: Diagnostic[] = [];
  let suppressed = 0;
  for (const f of findings) {
    const info = codeInfo(f.code);
    const owner = packages.ownerOf(f.file);
    if (owner && PLATFORM.has(owner.name)) continue;
    const file = rel(f.file);
    if (!info.fatal && suppress.matches(f.code, file, owner?.name ?? null)) {
      suppressed++;
      continue;
    }
    diagnostics.push({
      code: f.code,
      severity: info.severity,
      message: f.message,
      file,
      package: owner ? { name: owner.name, version: owner.version } : null,
      range: f.range,
      help: f.help ?? info.fix,
      related: f.related.map((r) => ({ file: rel(r.file), range: r.range, message: r.message })),
    });
  }
  diagnostics.sort(
    (a, b) =>
      (a.file < b.file ? -1 : a.file > b.file ? 1 : 0) ||
      a.range.start.line - b.range.start.line ||
      a.range.start.character - b.range.start.character ||
      a.code.localeCompare(b.code),
  );
  return { diagnostics, suppressed };
}

/** One line per diagnostic, for warnings that do not stop a build. */
export function diagnosticLine(d: Diagnostic): string {
  return `${d.file}:${d.range.start.line + 1}:${d.range.start.character + 1} ${d.code} ${d.message}`;
}
