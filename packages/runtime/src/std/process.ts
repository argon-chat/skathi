// `skathi:process`: the programs sk.config.json lists under "permissions.process", started without a
// shell. The host checks the list, starts the program and owns its streams.

export type Stdio = "pipe" | "ignore";

export interface SpawnOptions {
  /** An absolute path to an existing folder. Default: the app's working folder. */
  cwd?: string;
  /** Set over the app's environment. PATH, LD_*, DYLD_* and other loader variables are refused. */
  env?: Record<string, string>;
  /** Default `"pipe"`. */
  stdin?: Stdio;
  stdout?: Stdio;
  stderr?: Stdio;
  /** Aborting it is `kill()`. */
  signal?: AbortSignal;
}

export interface ExecOptions extends SpawnOptions {
  /** Milliseconds; the process is killed and the promise rejects with `TimeoutError`. 0: none. */
  timeout?: number;
  /** Bytes kept of stdout and of stderr each (default 1 MiB); past it the process is killed. */
  maxBuffer?: number;
}

export interface ExecResult {
  /** `null` when the process was killed. */
  readonly code: number | null;
  readonly signal: string | null;
  readonly stdout: string;
  readonly stderr: string;
}

export interface Process {
  readonly pid: number;
  readonly stdin: WritableStream<Uint8Array> | null;
  readonly stdout: ReadableStream<Uint8Array> | null;
  readonly stderr: ReadableStream<Uint8Array> | null;
  /** The exit code, or `null` when the process was killed. */
  readonly exited: Promise<number | null>;
  /** Ends the process and what it started. Default `"SIGTERM"`. */
  kill(signal?: "SIGTERM" | "SIGKILL"): void;
}

export interface ProcessConstructor {
  readonly prototype: Process;
  /** Starts a program from "permissions.process" (its name, or an absolute path an entry names). */
  spawn(program: string, args?: readonly string[], options?: SpawnOptions): Process;
  /** Starts it, waits for it to exit and answers with its output as UTF-8. */
  exec(program: string, args?: readonly string[], options?: ExecOptions): Promise<ExecResult>;
}

declare const __sk_std: { process: { Process: ProcessConstructor } };

export const Process: ProcessConstructor = __sk_std.process.Process;
