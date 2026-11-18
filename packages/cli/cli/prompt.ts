// Terminal questions for `sk create`: text, select, multiselect and confirm, drawn in place on a TTY.
import { colorEnabled, paint, unicodeTerminal } from "./term";

export const CANCEL = Symbol("cancel");
export type Answer<T> = T | typeof CANCEL;

export type Key =
  | { name: "up" | "down" | "left" | "right" | "enter" | "space" | "backspace" | "tab" | "cancel" }
  | { name: "char"; char: string };

/** The keys in one chunk of raw stdin; a paste arrives as many `char` keys. */
export function parseKeys(chunk: string): Key[] {
  const keys: Key[] = [];
  for (let i = 0; i < chunk.length; i++) {
    const c = chunk[i] ?? "";
    if (c === "\x1b") {
      const seq = /^\x1b(?:\[[0-9;]*[A-Za-z~]|O[A-Za-z])/.exec(chunk.slice(i))?.[0];
      if (!seq) {
        keys.push({ name: "cancel" });
        continue;
      }
      i += seq.length - 1;
      const end = seq.at(-1);
      if (end === "A") keys.push({ name: "up" });
      else if (end === "B") keys.push({ name: "down" });
      else if (end === "C") keys.push({ name: "right" });
      else if (end === "D") keys.push({ name: "left" });
      continue;
    }
    if (c === "\r" || c === "\n") keys.push({ name: "enter" });
    else if (c === "\x03" || c === "\x04") keys.push({ name: "cancel" });
    else if (c === "\x7f" || c === "\b") keys.push({ name: "backspace" });
    else if (c === "\t") keys.push({ name: "tab" });
    else if (c === " ") keys.push({ name: "space" });
    else if (c >= " ") keys.push({ name: "char", char: c });
  }
  return keys;
}

const color = paint(colorEnabled(process.stdout));
const uni = unicodeTerminal();
const g = uni
  ? { active: "◆", done: "◇", cancel: "■", bar: "│", start: "┌", end: "└", on: "●", off: "○", boxOn: "◼", boxOff: "◻" }
  : {
      active: "*",
      done: "o",
      cancel: "x",
      bar: "|",
      start: "T",
      end: "-",
      on: ">",
      off: " ",
      boxOn: "[+]",
      boxOff: "[ ]",
    };

const visible = (line: string) => line.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "").length;
const out = (text: string) => process.stdout.write(text);

export function intro(title: string): void {
  out(`${color.dim(g.start)}  ${color.bold(title)}\n${color.dim(g.bar)}\n`);
}

export function outro(message: string): void {
  out(`${color.dim(g.end)}  ${message}\n\n`);
}

export function note(body: string, title: string): void {
  const lines = body.split("\n");
  out(`${color.green(g.done)}  ${title}\n`);
  for (const line of lines) out(`${color.dim(g.bar)}  ${color.dim(line)}\n`);
  out(`${color.dim(g.bar)}\n`);
}

export function cancel(message: string): void {
  out(`${color.dim(g.end)}  ${color.red(message)}\n\n`);
}

/** What a key did: answered, refused the answer with a reason, or nothing yet. */
type Step<T> = { answer: T } | { error: string } | undefined;

/**
 * Draws `view` and redraws it after every key until `handle` answers. The finished question collapses
 * to its message and the answer `summary` gives.
 */
function ask<T>(
  message: string,
  view: (error: string) => string[],
  handle: (key: Key) => Step<T>,
  summary: (value: T) => string,
): Promise<Answer<T>> {
  const stdin = process.stdin;
  let rows = 0;
  let error = "";
  const draw = (lines: string[]) => {
    const columns = process.stdout.columns || 80;
    if (rows > 1) out(`\x1b[${rows - 1}A`);
    out(`\r\x1b[J${lines.join("\n")}`);
    rows = lines.reduce((n, line) => n + Math.max(1, Math.ceil(visible(line) / columns)), 0);
  };
  const frame = () => [`${color.cyan(g.active)}  ${message}`, ...view(error).map((l) => `${color.cyan(g.bar)}  ${l}`)];
  return new Promise((resolve) => {
    const finish = (value: Answer<T>) => {
      stdin.off("data", onData);
      stdin.setRawMode(false);
      stdin.pause();
      const done =
        value === CANCEL
          ? `${color.red(g.cancel)}  ${message}\n${color.dim(g.bar)}`
          : `${color.green(g.done)}  ${message}\n${color.dim(g.bar)}  ${color.dim(summary(value))}\n${color.dim(g.bar)}`;
      draw([done]);
      out("\n\x1b[?25h");
      resolve(value);
    };
    const onData = (chunk: Buffer | string) => {
      for (const key of parseKeys(chunk.toString())) {
        if (key.name === "cancel") return finish(CANCEL);
        const step = handle(key);
        if (step && "answer" in step) return finish(step.answer);
        error = step?.error ?? "";
      }
      draw(frame());
    };
    stdin.setRawMode(true);
    stdin.resume();
    stdin.on("data", onData);
    out("\x1b[?25l");
    draw(frame());
  });
}

const failed = (error: string) => (error ? [color.yellow(error)] : []);

export function text(options: {
  message: string;
  placeholder?: string;
  initial?: string;
  /** What an empty answer means. */
  fallback?: string;
  /** A message when the answer is not acceptable. */
  validate?: (value: string) => string | undefined;
}): Promise<Answer<string>> {
  let value = options.initial ?? "";
  return ask<string>(
    options.message,
    (error) => [
      value ? `${value}${color.dim("█")}` : color.dim(`${options.placeholder ?? ""}█`.trimStart()),
      ...failed(error),
    ],
    (key) => {
      if (key.name === "char") value += key.char;
      else if (key.name === "space") value += " ";
      else if (key.name === "backspace") value = value.slice(0, -1);
      else if (key.name === "enter") {
        const answer = value.trim() || options.fallback || "";
        const error = options.validate?.(answer);
        return error ? { error } : { answer };
      }
      return undefined;
    },
    (answer) => answer,
  );
}

interface Choice<T> {
  value: T;
  label: string;
  hint?: string;
}

const choiceLine = <T>(choice: Choice<T>, mark: string, current: boolean) =>
  `${mark} ${current ? choice.label : color.dim(choice.label)}${current && choice.hint ? color.dim(`  ${choice.hint}`) : ""}`;

export function select<T>(options: { message: string; options: Choice<T>[]; initial?: T }): Promise<Answer<T>> {
  const choices = options.options;
  let at = Math.max(
    0,
    choices.findIndex((c) => c.value === options.initial),
  );
  return ask<T>(
    options.message,
    () => choices.map((c, i) => choiceLine(c, i === at ? color.green(g.on) : color.dim(g.off), i === at)),
    (key) => {
      if (key.name === "up" || key.name === "left") at = (at + choices.length - 1) % choices.length;
      else if (key.name === "down" || key.name === "right" || key.name === "tab") at = (at + 1) % choices.length;
      else if (key.name === "enter") {
        const choice = choices[at];
        if (choice) return { answer: choice.value };
      }
      return undefined;
    },
    (value) => choices.find((c) => c.value === value)?.label ?? String(value),
  );
}

export function multiselect<T>(options: {
  message: string;
  options: Choice<T>[];
  initial?: T[];
  required?: boolean;
}): Promise<Answer<T[]>> {
  const choices = options.options;
  const picked = new Set(options.initial ?? []);
  let at = 0;
  return ask<T[]>(
    options.message,
    (error) => [
      ...choices.map((c, i) =>
        choiceLine(c, picked.has(c.value) ? color.green(g.boxOn) : color.dim(g.boxOff), i === at),
      ),
      error ? color.yellow(error) : color.dim("space to pick, enter to go on"),
    ],
    (key) => {
      if (key.name === "up" || key.name === "left") at = (at + choices.length - 1) % choices.length;
      else if (key.name === "down" || key.name === "right" || key.name === "tab") at = (at + 1) % choices.length;
      else if (key.name === "space") {
        const value = choices[at]?.value;
        if (value !== undefined && !picked.delete(value)) picked.add(value);
      } else if (key.name === "enter") {
        if (options.required && picked.size === 0) return { error: "pick at least one" };
        return { answer: choices.filter((c) => picked.has(c.value)).map((c) => c.value) };
      }
      return undefined;
    },
    (values) =>
      choices
        .filter((c) => values.includes(c.value))
        .map((c) => c.label)
        .join(", "),
  );
}

export function confirm(options: { message: string; initial?: boolean }): Promise<Answer<boolean>> {
  let yes = options.initial ?? false;
  return ask<boolean>(
    options.message,
    () => [
      `${yes ? color.green(g.on) : color.dim(g.off)} ${yes ? "Yes" : color.dim("Yes")}  ${
        yes ? color.dim(g.off) : color.green(g.on)
      } ${yes ? color.dim("No") : "No"}`,
    ],
    (key) => {
      if (key.name === "left" || key.name === "right" || key.name === "up" || key.name === "down" || key.name === "tab")
        yes = !yes;
      else if (key.name === "char" && /[yn]/i.test(key.char)) return { answer: key.char.toLowerCase() === "y" };
      else if (key.name === "enter") return { answer: yes };
      return undefined;
    },
    (value) => (value ? "Yes" : "No"),
  );
}
