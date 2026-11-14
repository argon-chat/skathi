import type ts from "typescript";
import type { Range } from "./schema";

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
