export interface Paint {
  red: (text: string) => string;
  green: (text: string) => string;
  yellow: (text: string) => string;
  blue: (text: string) => string;
  magenta: (text: string) => string;
  cyan: (text: string) => string;
  bold: (text: string) => string;
  dim: (text: string) => string;
  on: boolean;
}

/** FORCE_COLOR and NO_COLOR win over the terminal check; an explicit flag wins over both. */
export function colorEnabled(stream: { isTTY?: boolean }, flag?: boolean): boolean {
  if (flag !== undefined) return flag;
  const force = process.env.FORCE_COLOR;
  if (force !== undefined) return force !== "0" && force !== "false";
  if (process.env.NO_COLOR) return false;
  return stream.isTTY === true && process.env.TERM !== "dumb";
}

/** `--color` / `--no-color`; undefined when neither is given. */
export const colorFlag = (flags: { color?: unknown; "no-color"?: unknown }) =>
  flags["no-color"] === true ? false : flags.color === true ? true : undefined;

export function paint(on: boolean): Paint {
  const sgr = (open: string) => (text: string) => (on && text ? `\x1b[${open}m${text}\x1b[0m` : text);
  return {
    red: sgr("1;31"),
    green: sgr("32"),
    yellow: sgr("1;33"),
    blue: sgr("1;34"),
    magenta: sgr("35"),
    cyan: sgr("36"),
    bold: sgr("1"),
    dim: sgr("2"),
    on,
  };
}

export const seconds = (ms: number) => (ms < 1000 ? `${Math.round(ms)}ms` : `${(ms / 1000).toFixed(1)}s`);

export const mib = (bytes: number) => `${(bytes / 2 ** 20).toFixed(1)} MiB`;

/** False on the legacy Windows console, whose fonts have no Braille or check marks. */
export function unicodeTerminal(env: NodeJS.ProcessEnv = process.env, platform: string = process.platform): boolean {
  if (platform !== "win32") return env.TERM !== "linux";
  return Boolean(
    env.WT_SESSION ||
    env.TERM_PROGRAM === "vscode" ||
    env.TERMINAL_EMULATOR === "JetBrains-JediTerm" ||
    env.ConEmuTask === "{cmd::Cmder}" ||
    env.TERM === "xterm-256color" ||
    env.TERM === "alacritty",
  );
}

/** A terminal that can take a redrawn line: a TTY, not TERM=dumb, not a CI log that merely has a pty. */
export function interactive(stream: { isTTY?: boolean }, env: NodeJS.ProcessEnv = process.env): boolean {
  const ci = env.CI !== undefined && env.CI !== "" && env.CI !== "0" && env.CI !== "false";
  return stream.isTTY === true && env.TERM !== "dumb" && !ci;
}

export interface Glyphs {
  ok: string;
  fail: string;
  frames: string[];
  done: string;
  todo: string;
  more: string;
}

export function glyphs(unicode: boolean): Glyphs {
  return unicode
    ? {
        ok: "✓",
        fail: "✗",
        frames: ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
        done: "━",
        todo: "─",
        more: "…",
      }
    : { ok: "+", fail: "x", frames: ["-", "\\", "|", "/"], done: "#", todo: "-", more: "~" };
}

export interface Progress {
  /** Starts a phase (ending the previous one); `total` shows a counter that `tick` advances. */
  step(label: string, total?: number): void;
  tick(by?: number): void;
  /** A download's position: a bar on a terminal. */
  bytes(done: number, total: number): void;
  /** What the finished phase line says after its label. */
  detail(text: string): void;
  /** A line printed above the spinner. */
  info(text: string): void;
  warn(text: string): void;
  /** Ends the current phase as failed, then everything. */
  fail(): void;
  end(): void;
}

export const quietProgress: Progress = {
  step() {},
  tick() {},
  bytes() {},
  detail() {},
  info() {},
  warn() {},
  fail() {},
  end() {},
};

/** Progress as plain lines (CI logs, pipes): each finished phase is one line. */
export function lineProgress(write: (line: string) => void, color: boolean, unicode = true): Progress {
  const p = paint(color);
  const g = glyphs(unicode);
  let label: string | null = null;
  let note = "";
  let began = 0;
  const finish = (ok: boolean) => {
    if (label === null) return;
    const mark = ok ? p.green(g.ok) : p.red(g.fail);
    write(`${mark} ${label}${note ? ` ${p.dim(note)}` : ""} ${p.dim(seconds(performance.now() - began))}`);
    label = null;
  };
  return {
    step(next) {
      finish(true);
      label = next;
      note = "";
      began = performance.now();
    },
    tick() {},
    bytes() {},
    detail(text) {
      note = text;
    },
    info(text) {
      write(`  ${text}`);
    },
    warn(text) {
      write(`${p.yellow("warning")}: ${text}`);
    },
    fail: () => finish(false),
    end: () => finish(true),
  };
}

/** The spinner on an interactive terminal, plain lines otherwise. */
export function progressFor(stream: NodeJS.WriteStream, color = colorEnabled(stream)): Progress {
  return interactive(stream)
    ? spinner(stream, color)
    : lineProgress((line) => stream.write(`${line}\n`), color, !stream.isTTY || unicodeTerminal());
}

/** A `log(line)` callback onto a progress: "warning: …" lines become warnings. */
export function logTo(progress: Progress): (line: string) => void {
  return (line) => {
    const text = line.replace(/^\[sk\] /, "");
    if (text.startsWith("warning: ")) progress.warn(text.slice("warning: ".length));
    else progress.info(text);
  };
}

const SIGNALS: [NodeJS.Signals, number][] = [
  ["SIGINT", 130],
  ["SIGTERM", 143],
  ["SIGHUP", 129],
  ...(process.platform === "win32" ? [["SIGBREAK", 149] as [NodeJS.Signals, number]] : []),
];

/**
 * A one-line spinner on a terminal stream; phases leave a ✓ (or ✗) line with their time. The line is
 * redrawn in one write and cut to the terminal's width; console output while it spins lands above it.
 */
export function spinner(stream: NodeJS.WriteStream, color: boolean, unicode = unicodeTerminal()): Progress {
  const p = paint(color);
  const g = glyphs(unicode);
  let label: string | null = null;
  let note = "";
  let total = 0;
  let done = 0;
  let download: { done: number; total: number } | null = null;
  let began = 0;
  let shown = false;
  let drawn = 0;
  let over = false;

  const bar = (part: number, whole: number, width: number) => {
    const filled = whole > 0 ? Math.min(width, Math.round((part / whole) * width)) : 0;
    return p.cyan(g.done.repeat(filled)) + p.dim(g.todo.repeat(width - filled));
  };
  const line = () => {
    const now = performance.now();
    const frame = g.frames[Math.floor((now - began) / 80) % g.frames.length]!;
    const time = seconds(now - began);
    const counter = download
      ? ` ${(download.done / 2 ** 20).toFixed(1)}/${(download.total / 2 ** 20).toFixed(1)} MiB`
      : total
        ? ` ${done}/${total}`
        : "";
    const width = Math.max(20, (stream.columns || 80) - 1);
    const meter = download && width >= 60 ? 20 : 0;
    const fixed = 2 + counter.length + 1 + time.length + (meter ? meter + 1 : 0);
    const fit = (text: string, room: number) =>
      text.length <= room ? text : text.slice(0, Math.max(0, room - g.more.length)) + g.more;
    if (fixed + 1 + g.more.length > width) return `${p.cyan(frame)} ${fit(label ?? "", width - 2)}`;
    const text = fit(label ?? "", width - fixed);
    const drawnBar = meter && download ? ` ${bar(download.done, download.total, meter)}` : "";
    return `${p.cyan(frame)} ${text}${drawnBar}${p.dim(counter)} ${p.dim(time)}`;
  };
  const draw = () => {
    if (label === null || over) return;
    drawn = performance.now();
    stream.write(`\r${line()}\x1b[K`);
    shown = true;
  };
  const clear = () => {
    if (!shown) return;
    stream.write("\r\x1b[K");
    shown = false;
  };
  const finish = (ok: boolean) => {
    if (label === null) return;
    clear();
    const mark = ok ? p.green(g.ok) : p.red(g.fail);
    const what = note ? ` ${p.dim(note)}` : "";
    stream.write(`${mark} ${label}${what} ${p.dim(seconds(performance.now() - began))}\n`);
    label = null;
  };

  const consoleBefore = { log: console.log, info: console.info, warn: console.warn, error: console.error };
  const above =
    (original: ((...data: unknown[]) => void) | undefined) =>
    (...data: unknown[]) => {
      clear();
      original?.(...data);
      draw();
    };
  console.log = above(consoleBefore.log);
  console.info = above(consoleBefore.info);
  console.warn = above(consoleBefore.warn);
  console.error = above(consoleBefore.error);
  const showCursor = () => stream.write("\x1b[?25h");
  const onSignal = SIGNALS.map(([signal, code]) => {
    const handler = () => {
      clear();
      showCursor();
      process.exit(code);
    };
    process.once(signal, handler);
    return [signal, handler] as const;
  });
  stream.write("\x1b[?25l");
  process.once("exit", showCursor);
  const timer = setInterval(draw, 80);
  timer.unref();

  const end = () => {
    if (over) return;
    finish(true);
    over = true;
    clearInterval(timer);
    Object.assign(console, consoleBefore);
    for (const [signal, handler] of onSignal) process.off(signal, handler);
    showCursor();
    process.off("exit", showCursor);
  };
  return {
    step(next, count = 0) {
      finish(true);
      label = next;
      note = "";
      total = count;
      done = 0;
      download = null;
      began = performance.now();
      draw();
    },
    tick(by = 1) {
      done += by;
      if (performance.now() - drawn > 50) draw();
    },
    bytes(at, of) {
      download = { done: at, total: of };
      if (performance.now() - drawn > 50) draw();
    },
    detail(text) {
      note = text;
    },
    info(text) {
      clear();
      stream.write(`  ${text}\n`);
      draw();
    },
    warn(text) {
      clear();
      stream.write(`${p.yellow("warning")}: ${text}\n`);
      draw();
    },
    fail() {
      finish(false);
      end();
    },
    end,
  };
}
