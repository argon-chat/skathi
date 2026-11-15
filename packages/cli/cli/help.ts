// `sk --help` and `sk <command> --help`, laid out from the commander declarations in sk.ts: usage,
// what the command does, its subcommands, and its options under their group headings.

import type { Argument, CommandUnknownOpts, Help } from "@commander-js/extra-typings";
import { paint, type Paint } from "./term";

export type Row = [string, string];

export function table(p: Paint, rows: Row[]): string[] {
  const width = Math.max(...rows.map(([term]) => term.length));
  return rows.map(([term, text]) => `  ${p.cyan(term.padEnd(width))}  ${text}`);
}

export const section = (p: Paint, title: string, rows: Row[]): string[] =>
  rows.length ? ["", p.bold(title), ...table(p, rows)] : [];

const argTerm = (a: Argument) => {
  const name = `${a.name()}${a.variadic ? "..." : ""}`;
  return a.required ? `<${name}>` : `[${name}]`;
};

function path(cmd: CommandUnknownOpts): string {
  const names: string[] = [];
  for (let c: CommandUnknownOpts | null = cmd; c; c = c.parent) names.unshift(c.name());
  return names.join(" ");
}

const visible = (helper: Help, cmd: CommandUnknownOpts) =>
  helper.visibleCommands(cmd).filter((c) => c.name() !== "help");

export function helpFormat(color: boolean): Partial<Help> {
  const p = paint(color);
  return {
    formatHelp(cmd, helper) {
      const commands = visible(helper, cmd);
      const options = helper.visibleOptions(cmd);
      const usage = [
        path(cmd),
        ...cmd.registeredArguments.map(argTerm),
        ...(commands.length ? ["<command>"] : []),
        ...(options.length ? ["[options]"] : []),
      ].join(" ");
      const lines: string[] = [];
      if (!cmd.parent) lines.push(`${p.bold(cmd.name())} ${p.dim(cmd.version() ?? "")}  ${cmd.description()}`, "");
      lines.push(`${p.bold("Usage")}  ${usage}`);
      if (cmd.parent && cmd.description()) lines.push("", cmd.description());
      lines.push(
        ...section(
          p,
          "Commands",
          commands.map((c) => [
            [c.name(), ...c.registeredArguments.map(argTerm), ...(visible(helper, c).length ? ["<command>"] : [])].join(
              " ",
            ),
            c.summary() || c.description(),
          ]),
        ),
      );
      const groups = new Map<string, Row[]>();
      for (const o of options) {
        const title = (o.helpGroupHeading ?? "Options").replace(/:$/, "");
        groups.set(title, [...(groups.get(title) ?? []), [o.flags, o.description]]);
      }
      for (const [title, rows] of groups) lines.push(...section(p, title, rows));
      return `${lines.join("\n")}\n`;
    },
  };
}
