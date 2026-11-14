// The JSON `sk check --format json` prints: schema "skathi.check", version 1. Additive changes keep the
// version; a renamed or removed field bumps it.

export type Severity = "error" | "warning";

/** Zero-based, in UTF-16 code units (LSP positions). */
export interface Position {
  line: number;
  character: number;
}

export interface Range {
  start: Position;
  end: Position;
}

export interface PackageRef {
  name: string;
  version: string;
}

export interface Related {
  /** Relative to `root`, with forward slashes. */
  file: string;
  range: Range;
  message: string;
}

export interface Diagnostic {
  code: string;
  severity: Severity;
  message: string;
  /** Relative to `root`, with forward slashes. */
  file: string;
  /** The package `file` belongs to; null for the app's own code. */
  package: PackageRef | null;
  range: Range;
  help: string | null;
  related: Related[];
}

export interface TypingScore {
  /** null for the app's own code. */
  package: string | null;
  version: string | null;
  /** Exported declarations the app reaches (for the app: its own declarations). */
  declarations: number;
  /** How many of them reach the compiler as `any`. */
  any: number;
}

export interface Summary {
  errors: number;
  warnings: number;
  suppressed: number;
  files: number;
  packages: number;
  typing: TypingScore[];
}

export interface CheckReport {
  schema: "skathi.check";
  version: 1;
  tool: { name: "sk"; version: string };
  /** The app directory, absolute, with forward slashes. */
  root: string;
  diagnostics: Diagnostic[];
  summary: Summary;
}
