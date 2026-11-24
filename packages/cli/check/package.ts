import type { AppConfig } from "../cli/config";
import { packageProblems } from "../cli/package/config";
import { currentPlatform } from "../cli/platform";
import { ZERO_RANGE, type Finding } from "./finding";
import type { Range } from "./schema";

/** Where `key` (`package.windows.msix.publisher`) is written: its last name, searched after the names before it. */
export function keyRange(text: string, key: string): Range {
  let at = 0;
  let found = -1;
  let name = "";
  for (const part of key.split(".")) {
    name = JSON.stringify(part.replace(/\[\d+\]$/, ""));
    const next = text.indexOf(name, at);
    if (next < 0) break;
    found = next;
    at = next + name.length;
  }
  if (found < 0) return ZERO_RANGE;
  const before = text.slice(0, found).split("\n");
  const line = before.length - 1;
  const character = before[line]!.length;
  return { start: { line, character }, end: { line, character: character + name.length } };
}

/** SK8001/SK8002 for sk.config.json "package", for the platforms the app ships to; nothing when it has none. */
export function packageFindings(config: AppConfig, configFile: string, configText: string): Finding[] {
  if (config.raw.package === undefined) return [];
  const { problems } = packageProblems(config, config.platforms ?? [currentPlatform()]);
  return problems.map((p) => ({
    code: p.code,
    message: `sk.config.json ${p.message}`,
    file: configFile,
    range: keyRange(configText, p.key),
    related: [],
  }));
}
