import { readFileSync } from "node:fs";
import { join } from "node:path";
import { paint, type Paint } from "../cli/term";
import { ALL_CODES, codeInfo, RANGES, RETIRED } from "./codes";
import type { Highlight } from "./highlight";
import type { CheckReport, Diagnostic, TypingScore } from "./schema";

const MAX_NOTES = 5;
const MAX_LINE = 120;
const TABLE_AFTER = 10;
const TYPING_ROWS = 15;

const plain: Highlight = (code) => code;
const same = (text: string) => text;

/** `code` spans in cyan, the rest through `base`. */
const inline = (p: Paint, text: string, base: (text: string) => string = same) =>
  text
    .split(/(`[^`]+`)/)
    .map((part) => (part.length > 1 && part.startsWith("`") ? p.cyan(part) : base(part)))
    .join("");

const at = (p: Paint, file: string, line: number, character: number) =>
  `${p.cyan(file)}${p.dim(`:${line + 1}:${character + 1}`)}`;

const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export const packageLabel = (d: Pick<Diagnostic, "package">) =>
  d.package ? `${d.package.name}@${d.package.version}` : "app";

function excerpt(text: string | null, d: Diagnostic, width: number, p: Paint, highlight: Highlight): string[] {
  const { start, end } = d.range;
  if (text === null || (start.line === 0 && start.character === 0 && end.line === 0 && end.character === 0)) return [];
  const line = text.split(/\r?\n/)[start.line];
  if (line === undefined) return [];
  const last = end.line === start.line ? end.character : line.length;
  let from = 0;
  let to = line.length;
  if (line.length > MAX_LINE) {
    from = Math.max(0, start.character - 40);
    to = Math.min(line.length, from + MAX_LINE);
  }
  const shown = `${from > 0 ? p.dim("…") : ""}${highlight(line.slice(from, to))}${to < line.length ? p.dim("…") : ""}`;
  const pad = `${from > 0 ? " " : ""}${line.slice(from, start.character).replace(/[^\t]/g, " ")}`;
  const marks = "^".repeat(Math.max(1, Math.min(last, to) - start.character));
  const bar = p.blue(`${" ".repeat(width + 1)}|`);
  return [
    bar,
    `${p.blue(`${String(start.line + 1).padStart(width)} |`)} ${shown}`,
    `${bar} ${pad}${d.severity === "error" ? p.red(marks) : p.yellow(marks)}`,
    bar,
  ];
}

export function formatDiagnostic(
  d: Diagnostic,
  source: string | null,
  color: boolean,
  highlight: Highlight = plain,
): string {
  const p = paint(color);
  const width = String(d.range.start.line + 1).length;
  const head = d.severity === "error" ? p.red(`error[${d.code}]`) : p.yellow(`warning[${d.code}]`);
  const pkg = d.package ? ` ${p.magenta(`(${packageLabel(d)})`)}` : "";
  const lines = [
    `${head}${p.bold(":")} ${inline(p, d.message, p.bold)}`,
    `${" ".repeat(width)}${p.blue("-->")} ${at(p, d.file, d.range.start.line, d.range.start.character)}${pkg}`,
    ...excerpt(source, d, width, p, highlight),
  ];
  const gutter = `${" ".repeat(width + 1)}${p.blue("=")}`;
  const note = (text: string) => `${gutter} ${p.bold("note")}: ${text}`;
  for (const r of d.related.slice(0, MAX_NOTES)) {
    const place = at(p, r.file, r.range.start.line, r.range.start.character);
    lines.push(note(`${inline(p, r.message)} ${p.dim("(")}${place}${p.dim(")")}`));
  }
  if (d.related.length > MAX_NOTES)
    lines.push(note(p.dim(`and ${d.related.length - MAX_NOTES} more (--format json lists all)`)));
  if (d.help) lines.push(`${gutter} ${p.green("help")}: ${inline(p, d.help)}`);
  lines.push(`${gutter} ${p.dim(`see: sk explain ${d.code}`)}`);
  return lines.join("\n");
}

type Cell = { text: string; paint?: (text: string) => string };

function table(rows: Cell[][], align: ("l" | "r")[]): string[] {
  const widths = align.map((_, i) => Math.max(...rows.map((r) => r[i]!.text.length)));
  return rows.map((r) =>
    r
      .map((cell, i) => {
        const text = align[i] === "r" ? cell.text.padStart(widths[i]!) : cell.text.padEnd(widths[i]!);
        return i === r.length - 1 && align[i] === "l"
          ? (cell.paint ?? same)(text.trimEnd())
          : (cell.paint ?? same)(text);
      })
      .join("  "),
  );
}

function typingSection(scores: TypingScore[], untyped: Set<string>, p: Paint): string[] {
  const label = (s: TypingScore) => (s.package ? `${s.package}@${s.version}` : "app");
  const rows = scores
    .filter((s) => s.declarations > 0)
    .map((s) => ({ s, share: untyped.has(label(s)) ? 0 : (s.declarations - s.any) / s.declarations }))
    .sort((a, b) => a.share - b.share || b.s.any - a.s.any || label(a.s).localeCompare(label(b.s)));
  if (!rows.length) return [];
  const grade = (share: number) => (share < 0.7 ? p.red : share < 0.9 ? p.yellow : p.green);
  const body = rows.slice(0, TYPING_ROWS).map(({ s, share }): Cell[] => [
    { text: `  ${label(s)}`, paint: s.package ? same : p.bold },
    untyped.has(label(s))
      ? { text: "untyped", paint: p.red }
      : { text: `${Math.floor(share * 100)}% typed`, paint: grade(share) },
    {
      text: untyped.has(label(s)) ? "no declarations" : `${s.any} of ${s.declarations} use any`,
      paint: p.dim,
    },
  ]);
  const out = ["", p.bold("typing quality (worst first)"), ...table(body, ["l", "r", "l"])];
  if (rows.length > TYPING_ROWS) out.push(p.dim(`  … ${rows.length - TYPING_ROWS} more (--format json lists all)`));
  return out;
}

export function formatPretty(report: CheckReport, options: { color: boolean; highlight?: Highlight }): string {
  const p = paint(options.color);
  const sources = new Map<string, string | null>();
  const source = (file: string) => {
    if (!sources.has(file)) {
      let text: string | null;
      try {
        text = readFileSync(join(report.root, file), "utf8").replace(/^﻿/, "");
      } catch {
        text = null;
      }
      sources.set(file, text);
    }
    return sources.get(file) ?? null;
  };
  const blocks = report.diagnostics.map((d) => formatDiagnostic(d, source(d.file), options.color, options.highlight));
  const s = report.summary;
  const counts = [plural(s.errors, "error"), plural(s.warnings, "warning")].join(", ");
  const suppressed = s.suppressed ? ` (${s.suppressed} suppressed)` : "";
  const totals = `${counts}${suppressed} in ${plural(s.files, "file")}, ${plural(s.packages, "package")}`;
  const tail: string[] = [s.errors ? p.red(totals) : s.warnings ? p.yellow(totals) : p.green(totals)];
  if (report.diagnostics.length > TABLE_AFTER) {
    const byKey = new Map<string, { code: string; pkg: string; n: number; error: boolean }>();
    for (const d of report.diagnostics) {
      const key = `${d.code} ${packageLabel(d)}`;
      const row = byKey.get(key) ?? { code: d.code, pkg: packageLabel(d), n: 0, error: d.severity === "error" };
      row.n++;
      byKey.set(key, row);
    }
    const rows = [...byKey.values()].sort(
      (a, b) => a.code.localeCompare(b.code) || b.n - a.n || a.pkg.localeCompare(b.pkg),
    );
    tail.push(
      "",
      ...table(
        [
          [
            { text: "code", paint: p.dim },
            { text: "package", paint: p.dim },
            { text: "count", paint: p.dim },
          ],
          ...rows.map((r): Cell[] => [
            { text: r.code, paint: r.error ? p.red : p.yellow },
            { text: r.pkg, paint: r.pkg === "app" ? p.bold : p.magenta },
            { text: String(r.n), paint: p.bold },
          ]),
        ],
        ["l", "l", "r"],
      ),
    );
  }
  const untyped = new Set(
    report.diagnostics.filter((d) => d.code === "SK3001" && d.package).map((d) => packageLabel(d)),
  );
  tail.push(...typingSection(s.typing, untyped, p));
  return [...blocks, tail.join("\n")].join("\n\n");
}

export function explain(code: string, options: { color?: boolean; highlight?: Highlight } = {}): string | null {
  const info = (() => {
    try {
      return codeInfo(code.toUpperCase());
    } catch {
      return null;
    }
  })();
  if (!info) {
    const retired = RETIRED.get(code.toUpperCase());
    return retired ? `${code.toUpperCase()} is retired: ${retired}, so sk check no longer reports it.` : null;
  }
  const p = paint(options.color ?? false);
  const highlight = options.highlight ?? plain;
  const severity = info.severity === "error" ? p.red : p.yellow;
  return [
    `${severity(info.code)} ${p.dim(`(${info.severity})`)}${p.dim(":")} ${inline(p, info.title, p.bold)}`,
    "",
    inline(p, info.explanation),
    "",
    p.bold("Example:"),
    ...info.example.split("\n").map((l) => `    ${highlight(l)}`),
    "",
    `${p.green("Fix:")} ${inline(p, info.fix)}`,
  ].join("\n");
}

export function listCodes(color = false): string {
  const p = paint(color);
  const lines: string[] = [];
  for (const range of RANGES) {
    const codes = ALL_CODES.filter((c) => Number(c.code.slice(2)) >= range.from && Number(c.code.slice(2)) <= range.to);
    if (!codes.length) continue;
    lines.push(
      p.bold(`SK${String(range.from).padStart(4, "0")}–SK${String(range.to).padStart(4, "0")}: ${range.what}`),
    );
    for (const c of codes) {
      const severity = c.severity === "error" ? p.red : p.yellow;
      lines.push(`  ${severity(c.code)}  ${p.dim(c.severity.padEnd(7))}  ${inline(p, c.title)}`);
    }
  }
  return lines.join("\n");
}
