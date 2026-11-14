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

export interface Progress {
  /** Starts a phase (ending the previous one); `total` shows a counter that `tick` advances. */
  step(label: string, total?: number): void;
  tick(): void;
  /** What the finished phase line says after its label. */
  detail(text: string): void;
  /** A line printed above the spinner. */
  info(text: string): void;
  warn(text: string): void;
  end(): void;
}

export const quietProgress: Progress = { step() {}, tick() {}, detail() {}, info() {}, warn() {}, end() {} };

/** Progress as plain lines (CI logs, pipes): each finished phase is one line. */
export function lineProgress(write: (line: string) => void, color: boolean): Progress {
  const p = paint(color);
  let label: string | null = null;
  let note = "";
  let began = 0;
  const finish = () => {
    if (label === null) return;
    write(`${p.green("✓")} ${label}${note ? ` ${p.dim(note)}` : ""} ${p.dim(seconds(performance.now() - began))}`);
    label = null;
  };
  return {
    step(next) {
      finish();
      label = next;
      note = "";
      began = performance.now();
    },
    tick() {},
    detail(text) {
      note = text;
    },
    info(text) {
      write(`  ${text}`);
    },
    warn(text) {
      write(`${p.yellow("warning")}: ${text}`);
    },
    end: finish,
  };
}

/** The spinner on a terminal, plain lines otherwise. */
export function progressFor(stream: NodeJS.WriteStream, color = colorEnabled(stream)): Progress {
  return stream.isTTY ? spinner(stream, color) : lineProgress((line) => stream.write(`${line}\n`), color);
}

/** A `log(line)` callback onto a progress: "warning: …" lines become warnings. */
export function logTo(progress: Progress): (line: string) => void {
  return (line) => {
    const text = line.replace(/^\[sk\] /, "");
    if (text.startsWith("warning: ")) progress.warn(text.slice("warning: ".length));
    else progress.info(text);
  };
}

const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/** A one-line spinner on a terminal stream; phases leave a ✓ line with their time. */
export function spinner(stream: NodeJS.WriteStream, color: boolean): Progress {
  const p = paint(color);
  let label: string | null = null;
  let note = "";
  let total = 0;
  let done = 0;
  let frame = 0;
  let began = 0;
  let drawn = 0;
  const clear = () => stream.write("\r\x1b[2K");
  const draw = () => {
    if (label === null) return;
    drawn = performance.now();
    frame = (frame + 1) % FRAMES.length;
    const count = total ? ` ${done}/${total}` : "";
    clear();
    stream.write(`${p.cyan(FRAMES[frame]!)} ${label}${p.dim(count)} ${p.dim(seconds(drawn - began))}`);
  };
  const finish = () => {
    if (label === null) return;
    clear();
    const what = note ? ` ${p.dim(note)}` : "";
    stream.write(`${p.green("✓")} ${label}${what} ${p.dim(seconds(performance.now() - began))}\n`);
    label = null;
  };
  const restore = () => stream.write("\x1b[?25h");
  const interrupt = () => {
    clear();
    restore();
    process.exit(130);
  };
  stream.write("\x1b[?25l");
  process.once("exit", restore);
  process.once("SIGINT", interrupt);
  const timer = setInterval(draw, 80);
  return {
    step(next, count = 0) {
      finish();
      label = next;
      note = "";
      total = count;
      done = 0;
      began = performance.now();
      draw();
    },
    tick() {
      done++;
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
    end() {
      finish();
      clearInterval(timer);
      restore();
      process.off("exit", restore);
      process.off("SIGINT", interrupt);
    },
  };
}
