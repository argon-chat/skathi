import { Process } from "skathi:process";

const TRICKY = [
  "",
  " ",
  "two words",
  '"',
  "\\",
  'a\\\\"b',
  "trailing\\",
  "%PATH%",
  "& calc | x > y",
  "$(id) `id`",
  "line\nbreak",
  "ünï 🙂",
];
const ECHO = "for await (const chunk of process.stdin) process.stdout.write(chunk)";

const failure = (run: () => unknown): string => {
  try {
    run();
    return "ok";
  } catch (e) {
    return (e as Error).name;
  }
};

async function roundTrip(size: number): Promise<{ length: number; sum: number }> {
  const child = Process.spawn("bun", ["-e", ECHO]);
  const input = new Uint8Array(size).map((_, i) => (i * 7) % 256);
  const writing = (async () => {
    const writer = child.stdin!.getWriter();
    for (let at = 0; at < size; at += 65_536) await writer.write(input.subarray(at, at + 65_536));
    await writer.close();
  })();
  let length = 0;
  let sum = 0;
  let i = 0;
  for await (const chunk of child.stdout!) {
    length += chunk.length;
    for (const byte of chunk) if (byte === (i++ * 7) % 256) sum++;
  }
  await writing;
  await child.exited;
  return { length, sum };
}

export async function processes() {
  const argv = await Process.exec("bun", ["-e", "console.log(JSON.stringify(process.argv.slice(1)))", ...TRICKY]);
  const exit = await Process.exec("bun", ["-e", "console.error('to stderr'); process.exit(3)"]);
  const sleeper = Process.spawn("bun", ["-e", "setTimeout(() => {}, 60000)"], { stdout: "ignore" });
  sleeper.kill();
  return {
    argv: JSON.parse(argv.stdout) as unknown,
    same: JSON.stringify(JSON.parse(argv.stdout)) === JSON.stringify(TRICKY),
    exit: [exit.code, exit.signal, exit.stderr.trim()],
    roundTrip: await roundTrip(4 * 1024 * 1024),
    killed: await sleeper.exited,
    unlisted: failure(() => Process.spawn("node")),
    path: failure(() => Process.spawn("bun", [], { env: { PATH: "/tmp" } })),
    relative: failure(() => Process.spawn("./bun")),
  };
}

/** A child that would outlive the app if nothing ended it. */
export function lingering(): number {
  return Process.spawn("bun", ["-e", "setTimeout(() => {}, 120000)"], { stdout: "ignore", stderr: "ignore" }).pid;
}
