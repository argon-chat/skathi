import { importLine, section } from "./shared";

const PERMISSION =
  "A program starts only if sk.config.json lists it under [`\"permissions.process\"`](/sk-config/#permissions), and only with the arguments its entry allows. The rule is the same under `sk dev`. An app embedded in another program starts what that program allows with `Builder::allow_process()` instead, and the `process` feature of the `skathi` crate must be on. Listing a program lets the app's code, and anything that runs in it, start that program with the user's rights: list only what the app needs, with the arguments it needs.";
const NO_SHELL =
  "There is no shell, ever. The arguments reach the program as the list you give, byte for byte: quotes, spaces, `%VAR%`, `$(…)`, `&` and newlines mean nothing. On Windows only `.exe` files start: a `.bat` or `.cmd` file would run through `cmd.exe`, which parses its arguments again, so it is refused. Outside Windows a file must be an executable or a script that starts with `#!`.";
const ARGS =
  '`"args"` of the entry decides what code may pass: left out, no arguments; a list, exactly that many, each equal to its string or matching its `{ "pattern" }` from start to end (`["status", { "pattern": "--porcelain(=v[12])?" }]`); `true`, any. With `true` the app\'s code chooses every argument, and many programs can be made to run other commands through theirs (`git -c core.sshCommand=…`, `ffmpeg` filters that write files). `sk check` warns about it (SK6004).';
const PATHS =
  '`"$APP/bin/tool"` is a file in the app\'s folder that `sk build` copies beside the executable. `"$SYSTEM/where"` is in System32 on Windows, and in `/usr/bin`, `/bin`, `/usr/sbin` or `/sbin` elsewhere. `"$PATH/git"` is the first match on PATH when the program starts, which whoever controls PATH can change (SK6004). An absolute path is used as it is. On Windows `.exe` is added to a path without an extension, and PATH and `$SYSTEM` are searched for `.exe` files only, never the current folder.';
const LIFETIME =
  "A child does not outlive the app: when the app quits, every child gets SIGTERM and, half a second later, SIGKILL. A `Process` whose object is collected is killed too, so keep a reference (or one of its streams) while it runs. On Windows each child runs in a job object of its own, which also ends what it started; on Linux a child that loses the app is killed by the kernel even after a crash; on macOS nothing ends it after a crash.";

export const process = section("skathi:process", [
  {
    slug: "process-spawn",
    name: "Process.spawn()",
    kind: "function",
    summary: "Starts a program sk.config.json allows and returns it at once, with its input and output as streams.",
    syntax: `${importLine("skathi:process", "Process")}

static Process.spawn(program: string, args?: string[], options?: SpawnOptions): Process

interface SpawnOptions {
  cwd?: string                     // absolute; default: the app's working folder
  env?: Record<string, string>     // set over the app's environment
  stdin?: "pipe" | "ignore"        // default "pipe"
  stdout?: "pipe" | "ignore"
  stderr?: "pipe" | "ignore"
  signal?: AbortSignal             // abort = kill()
}`,
    params: [
      {
        name: "program",
        type: "string",
        description:
          'A name from `"permissions.process"`, or an absolute path that one of its entries resolves to. The path is compared as the file it is: links followed, `..` removed, and on Windows without regard to case, short names or trailing dots.',
      },
      { name: "args", type: "string[]", description: "The arguments, as they reach the program. Default: none." },
      {
        name: "options",
        type: "SpawnOptions",
        description:
          '`env` cannot set PATH, PATHEXT, COMSPEC, SystemRoot, windir, `LD_*`, `DYLD_*` or the variables a shell reads at start (`BASH_ENV`, `ENV`, `IFS`, `BASH_FUNC_*`): they decide what the program loads before its own code. `"ignore"` connects the stream to the null device, and the property is `null`.',
      },
    ],
    returns: "A [`Process`](/std/process/).",
    notes: [
      PERMISSION,
      NO_SHELL,
      ARGS,
      PATHS,
      "The child inherits the app's environment without Skathi's own `SK_*` variables, and inside an AppImage without the image's variables and folders; `env` is set over that.",
      "`stdout` and `stderr` hold up to 256 KiB that JS has not read. Then the host stops reading, the system's pipe fills, and the program waits on its next write until you read: a chatty program cannot fill the app's memory. A stream nobody reads stops the program the same way, so read it, cancel it, or start with `\"ignore\"`.",
      LIFETIME,
      "`sk check` compares a literal name with the config: a name that is not listed is the error SK6002, an import in an app that lists nothing is SK6001, a name that is not a literal is the warning SK6003, and a package in node_modules that imports `skathi:process` is the warning SK6005.",
      "`appShell.openPath()` still refuses executables without `allowExecutable`; it opens a file the way the desktop would and gives no output or exit code. `skathi:process` is for running a program you list and talking to it.",
    ],
    edgeCases: [
      '`NotAllowedError`: the program is not in `"permissions.process"`, the list is empty, the arguments do not fit its entry, the file is a batch file, or `env` sets a variable that is refused. The message names the entry to add or what does not fit.',
      "`NotFoundError`: the entry's file is not there, or `cwd` is not a folder.",
      "`QuotaExceededError`: 32 processes of the app are already running.",
      "`AbortError`: `signal` is already aborted.",
      "`NotSupportedError`: the host was built without the `process` feature. Hosts that `sk build` uses have it.",
      "`TypeError`: an argument that is not a string or has a NUL character, an `env` name with `=`, a relative `cwd`.",
      "`OperationError`: the system could not start the program; the message has its words.",
    ],
    examples: [
      {
        title: "Read a program's output as it comes",
        lang: "ts",
        note: 'sk.config.json: "permissions": { "process": [{ "name": "ffmpeg", "path": "$APP/bin/ffmpeg", "args": true }] }',
        code: `import { Process } from "skathi:process";

const ffmpeg = Process.spawn("ffmpeg", ["-i", input, "-progress", "pipe:1", output]);
const decoder = new TextDecoder();
for await (const chunk of ffmpeg.stdout!) {
  for (const line of decoder.decode(chunk, { stream: true }).split("\\n")) {
    if (line.startsWith("out_time_ms=")) progress.value = Number(line.slice(12)) / 1000;
  }
}
const code = await ffmpeg.exited;`,
      },
      {
        title: "Write to its input",
        lang: "ts",
        note: 'sk.config.json: "permissions": { "process": [{ "name": "sort", "path": "$SYSTEM/sort" }] }',
        code: `import { Process } from "skathi:process";

const sort = Process.spawn("sort");
const writer = sort.stdin!.getWriter();
await writer.write(new TextEncoder().encode("pear\\napple\\nfig\\n"));
await writer.close();
let sorted = "";
const decoder = new TextDecoder();
for await (const chunk of sort.stdout!) sorted += decoder.decode(chunk, { stream: true });`,
      },
    ],
    related: ["process", "process-exec"],
    aliases: ["spawn", "child process", "child_process", "subprocess", "run a program", "command", "SpawnOptions"],
  },
  {
    slug: "process-exec",
    name: "Process.exec()",
    kind: "function",
    summary: "Runs a program sk.config.json allows to its end and answers with its exit code and its output as text.",
    syntax: `${importLine("skathi:process", "Process")}

static Process.exec(program: string, args?: string[], options?: ExecOptions): Promise<ExecResult>

interface ExecOptions extends SpawnOptions {
  timeout?: number     // milliseconds; 0 or left out: none
  maxBuffer?: number   // bytes of stdout and of stderr each; default 1 MiB
}

interface ExecResult {
  code: number | null      // null when the process was killed
  signal: string | null    // "SIGTERM", "SIGKILL", …
  stdout: string
  stderr: string
}`,
    params: [
      { name: "program", type: "string", description: "As for [`Process.spawn()`](/std/process-spawn/)." },
      { name: "args", type: "string[]", description: "As for [`Process.spawn()`](/std/process-spawn/)." },
      {
        name: "options",
        type: "ExecOptions",
        description:
          "As for `Process.spawn()`, with `timeout` and `maxBuffer`. Its stdin is the null device: the program reads an end of input at once.",
      },
    ],
    returns: "A promise of `{ code, signal, stdout, stderr }`, the output decoded as UTF-8.",
    notes: [
      PERMISSION,
      "The promise settles once the program has exited and both of its streams have ended. A program that exits with an error code still resolves: check `code`.",
      "On Windows the program's job ends when it exits, so a helper it left running is ended too and cannot hold the output open.",
      LIFETIME,
    ],
    edgeCases: [
      "Every error of `Process.spawn()` rejects the promise instead of throwing.",
      "`TimeoutError`: `timeout` passed; the process was killed.",
      "`RangeError`: stdout or stderr passed `maxBuffer`; the process was killed.",
      "`AbortError`: `signal` aborted; the process was killed.",
    ],
    examples: [
      {
        title: "Ask git for the state of a folder",
        lang: "ts",
        note: 'sk.config.json: "permissions": { "process": [{ "name": "git", "path": { "windows": "C:/Program Files/Git/cmd/git.exe", "macos": "/usr/bin/git", "linux": "/usr/bin/git" }, "args": ["-C", { "pattern": "[^-].*" }, "status", "--porcelain"] }] }',
        code: `import { Process } from "skathi:process";

const { code, stdout, stderr } = await Process.exec("git", ["-C", folder, "status", "--porcelain"], { timeout: 5000 });
if (code !== 0) throw new Error(stderr);
const changed = stdout.split("\\n").filter(Boolean);`,
      },
    ],
    related: ["process-spawn", "process"],
    aliases: ["exec", "execFile", "run and wait", "ExecOptions", "ExecResult", "maxBuffer", "timeout"],
  },
  {
    slug: "process",
    name: "Process",
    kind: "class",
    summary: "A started program: its pid, its input and output streams, a promise of its exit, and a way to end it.",
    syntax: `${importLine("skathi:process", "Process")}

readonly process.pid: number
readonly process.stdin: WritableStream<Uint8Array> | null
readonly process.stdout: ReadableStream<Uint8Array> | null
readonly process.stderr: ReadableStream<Uint8Array> | null
readonly process.exited: Promise<number | null>
process.kill(signal?: "SIGTERM" | "SIGKILL"): void`,
    notes: [
      "[`Process.spawn()`](/std/process-spawn/) makes it; there is nothing to construct.",
      'The streams are the web\'s `ReadableStream` and `WritableStream`: `for await`, `getReader()` and `pipeTo()` work. A stream is `null` when its option was `"ignore"`.',
      "`stdin` takes bytes (`Uint8Array`, any typed array or `ArrayBuffer`); pipe text through a `TextEncoderStream`. A write settles once the program has taken the bytes; closing the writer ends the program's input.",
      "`exited` resolves with the exit code, or with `null` when the process was killed.",
      "`kill()` ends the program and what it started, and does nothing once it has exited. On macOS and Linux it sends the signal to the program's process group; a program that started its own session escapes it. On Windows there are no signals: both end the job at once, and the process counts as killed by the signal you named.",
      LIFETIME,
    ],
    examples: [
      {
        title: "Stop a long run",
        lang: "ts",
        code: `import { Process } from "skathi:process";

const run = Process.spawn("render", ["--scene", scene], { stdout: "ignore" });
cancel.onclick = () => run.kill();
const code = await run.exited; // null when cancelled`,
      },
    ],
    related: ["process-spawn", "process-exec"],
    aliases: ["pid", "kill", "exited", "stdin", "stdout", "stderr", "SIGTERM", "SIGKILL"],
  },
]);
