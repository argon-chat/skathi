import { colorEnabled, colorFlag, paint, quietProgress, spinner, type Progress } from "../cli/term";
import { highlighter, type Highlight } from "./highlight";
import { exitCode, explain, formatPretty, listCodes, loadTypeScript, runCheck } from "./index";

export interface CheckFlags {
  format?: string | boolean | undefined;
  config?: string | boolean | undefined;
  "deny-warnings"?: string | boolean | undefined;
  "no-deps"?: string | boolean | undefined;
  color?: string | boolean | undefined;
  "no-color"?: string | boolean | undefined;
}

export interface Io {
  out: (text: string) => void;
  err: (text: string) => void;
  color: boolean;
  progress: () => Progress;
}

export const defaultIo = (flags: Pick<CheckFlags, "color" | "no-color"> = {}): Io => {
  const color = colorEnabled(process.stdout, colorFlag(flags));
  return {
    out: (text) => process.stdout.write(`${text}\n`),
    err: (text) => process.stderr.write(`${text}\n`),
    color,
    progress: () =>
      process.stderr.isTTY ? spinner(process.stderr, colorEnabled(process.stderr, colorFlag(flags))) : quietProgress,
  };
};

function codeHighlight(color: boolean, root: string): Highlight | undefined {
  if (!color) return undefined;
  try {
    return highlighter(loadTypeScript(root), paint(true));
  } catch {
    return undefined;
  }
}

/** `sk check [app]`: 0 clean, 1 errors (or warnings with --deny-warnings), 2 usage, configuration or internal failure. */
export async function checkCommand(
  target: string | undefined,
  flags: CheckFlags,
  toolVersion: string,
  io = defaultIo(flags),
) {
  const format = flags.format ?? "pretty";
  const p = paint(colorEnabled(process.stderr, colorFlag(flags)));
  const fail = (text: string) => io.err(`${p.red("error")}: ${text}`);
  if (format !== "pretty" && format !== "json") {
    fail("--format is pretty or json");
    return 2;
  }
  if (flags.config !== undefined && (typeof flags.config !== "string" || !flags.config.trim())) {
    fail("--config needs a path");
    return 2;
  }
  const progress = io.progress();
  try {
    const report = await runCheck({
      target: target ?? ".",
      configPath: typeof flags.config === "string" ? flags.config : undefined,
      deps: !flags["no-deps"],
      toolVersion,
      progress,
    });
    progress.end();
    if (format === "json") io.out(JSON.stringify(report, null, 2));
    else {
      const highlight = codeHighlight(io.color, report.root);
      io.out(formatPretty(report, { color: io.color, ...(highlight ? { highlight } : {}) }));
    }
    return exitCode(report, flags["deny-warnings"] === true);
  } catch (e) {
    progress.end();
    fail(e instanceof Error ? e.message : String(e));
    return 2;
  }
}

/** `sk explain <CODE>` and `sk explain --list`. */
export function explainCommand(
  code: string | undefined,
  list: boolean,
  flags: Pick<CheckFlags, "color" | "no-color"> = {},
  io = defaultIo(flags),
): number {
  if (list) {
    io.out(listCodes(io.color));
    return 0;
  }
  if (!code) {
    io.err("usage: sk explain <CODE> | sk explain --list");
    return 2;
  }
  const highlight = codeHighlight(io.color, process.cwd());
  const text = explain(code, { color: io.color, ...(highlight ? { highlight } : {}) });
  if (!text) {
    io.err(`sk explain: ${code} is not a check code (sk explain --list shows them)`);
    return 2;
  }
  io.out(text);
  return 0;
}
