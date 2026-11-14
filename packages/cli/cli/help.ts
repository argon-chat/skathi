import { OPTIONAL } from "./config";
import { CLI_VERSION } from "./host";
import { paint, type Paint } from "./term";

type Row = [string, string];
type Section = { title: string; rows: Row[] };

const COMMANDS: Row[] = [
  ["create [dir]", "Start a new app"],
  ["dev [app]", "Run the app with hot reload"],
  ["build [app]", "Debug build: devtools on, not minified"],
  ["publish [app]", "Release build to ship"],
  ["run [app]", "Build, then start the app"],
  ["check [app]", "What breaks under AOT, and how well the code is typed"],
  ["explain <code>", "What a check code means and how to fix it"],
  ["host <command>", "Manage the prebuilt engine hosts"],
];

const BUILD: Section[] = [
  {
    title: "Options",
    rows: [
      ["--platform <os>", "windows, macos or linux (default: this OS)"],
      ["--without <parts>", `Leave engine parts out: ${OPTIONAL.join(", ")}`],
      ["--out <dir>", "Output directory (default: <app>/dist/debug or dist/release)"],
      ["--define K=V", "A Vite-style define; repeatable"],
      ["--config <path>", "An sk.config.json outside the app"],
      ["--sign <identity>", "macOS: codesign identity (- for ad hoc)"],
      ["--no-sign", "macOS: leave the bundle unsigned"],
      ["--console", "Windows: keep the console window"],
      ["--json", "Print the result as JSON"],
    ],
  },
  {
    title: "Advanced",
    rows: [
      ["--features <parts>", "Take only these engine parts"],
      ["--host <exe>", "Stamp this host instead of the cached one"],
      ["--payload-only", "Write dist/.sk/app.skpk and stop"],
      ["--aot-inputs", "Keep the typed sources for a later AOT compile"],
    ],
  },
];

const HELP: Record<string, { usage: string; about: string; sections: Section[] }> = {
  create: {
    usage: "sk create [dir] [options]",
    about: "Start a new app. In a terminal it asks for whatever the options leave open.",
    sections: [
      {
        title: "Options",
        rows: [
          ["--name <name>", "The app's name (default: the directory's)"],
          ["--template <name>", "showcase (every feature) or minimal"],
          ["--os <list>", "What to package for: windows, macos, linux"],
          ["--without <parts>", "Engine parts to leave out"],
          ["--devtools", "Keep devtools (F12) in published builds"],
          ["--pm <name>", "bun, npm, pnpm or yarn"],
          ["-y, --yes", "Ask nothing, take the defaults"],
        ],
      },
    ],
  },
  dev: {
    usage: "sk dev [app] [options]",
    about: "Run the app with hot reload: edits to .vue, .ts and .css apply without a restart.",
    sections: [{ title: "Options", rows: [["--exe <host>", "Run this dev host instead of the cached one"]] }],
  },
  build: {
    usage: "sk build [app] [options]",
    about: "A debug build in <app>/dist/debug: devtools (F12) on, the bundle not minified.",
    sections: BUILD,
  },
  publish: {
    usage: "sk publish [app] [options]",
    about: "The release build in <app>/dist/release: minified, devtools only if sk.config.json keeps them.",
    sections: BUILD,
  },
  run: {
    usage: "sk run [app] [options] [-- app args]",
    about: "sk build, then start the app with its output in this terminal.",
    sections: BUILD,
  },
  check: {
    usage: "sk check [app] [options]",
    about: "What in the bundle, app code and node_modules, breaks or slows down under AOT, and how well it is typed.",
    sections: [
      {
        title: "Options",
        rows: [
          ["--format json", "One JSON report on stdout (schema skathi.check)"],
          ["--deny-warnings", "Fail on warnings too"],
          ["--no-deps", "Only the app's own code"],
          ["--config <path>", "An sk.config.json outside the app"],
        ],
      },
      {
        title: "Exit codes",
        rows: [
          ["0", "Clean"],
          ["1", "Errors (or warnings with --deny-warnings)"],
          ["2", "Bad usage or config, or the check failed"],
        ],
      },
    ],
  },
  explain: {
    usage: "sk explain <code> | --list",
    about: "What a check code means, an example, and how to fix it.",
    sections: [{ title: "Options", rows: [["--list", "Every code"]] }],
  },
  host: {
    usage: "sk host <command>",
    about: "The engine host is the prebuilt executable an app is packed into. sk build fetches it on demand.",
    sections: [
      {
        title: "Commands",
        rows: [
          ["fetch", "Download this version's hosts into the local cache"],
          ["path", "Which host sk build would use"],
          ["info [exe]", "What a host says about itself"],
          ["add <exe>", "Verify a host and add it to the cache"],
          ["build", "In a Skathi checkout: build the hosts with cargo"],
        ],
      },
    ],
  },
};

function rows(p: Paint, list: Row[]): string[] {
  const width = Math.max(...list.map(([name]) => name.length));
  return list.map(([name, text]) => `  ${p.cyan(name.padEnd(width))}  ${text}`);
}

function sections(p: Paint, list: Section[]): string[] {
  return list.flatMap((s) => ["", p.bold(s.title), ...rows(p, s.rows)]);
}

const head = (p: Paint) => `${p.bold("sk")} ${p.dim(CLI_VERSION)}  Skathi apps for Windows, macOS and Linux`;

export function topHelp(color: boolean): string {
  const p = paint(color);
  return [
    head(p),
    "",
    `${p.bold("Usage")}  sk <command> [app] [options]`,
    "",
    p.bold("Commands"),
    ...rows(p, COMMANDS),
    "",
    p.dim("Run sk <command> --help for its options. [app] is a directory (default: .)."),
  ].join("\n");
}

export function commandHelp(command: string, color: boolean): string | null {
  const help = HELP[command];
  if (!help) return null;
  const p = paint(color);
  return [`${p.bold("Usage")}  ${help.usage}`, "", help.about, ...sections(p, help.sections)].join("\n");
}
