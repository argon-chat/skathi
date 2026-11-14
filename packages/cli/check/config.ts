import { REGISTRY } from "./codes";

export interface CheckConfig {
  /** Code -> package names and app-relative globs whose diagnostics of that code are not reported. */
  allow: Map<string, string[]>;
  /** What is wrong, and the JSON key it is about. */
  problems: { message: string; key: string }[];
}

export function parseCheckConfig(raw: unknown): CheckConfig {
  const allow = new Map<string, string[]>();
  const problems: CheckConfig["problems"] = [];
  if (raw === undefined) return { allow, problems };
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    problems.push({
      message: '"check" must be an object, e.g. { "allow": { "SK2001": ["some-package"] } }',
      key: "check",
    });
    return { allow, problems };
  }
  for (const key of Object.keys(raw)) {
    if (key !== "allow" && !key.startsWith("$"))
      problems.push({ message: `"check": unknown key ${JSON.stringify(key)}; the known key is allow`, key });
  }
  const list = (raw as Record<string, unknown>).allow;
  if (list === undefined) return { allow, problems };
  if (!list || typeof list !== "object" || Array.isArray(list)) {
    problems.push({
      message: '"check.allow" must map codes to lists, e.g. { "SK2001": ["some-package", "src/legacy/**"] }',
      key: "allow",
    });
    return { allow, problems };
  }
  for (const [code, value] of Object.entries(list as Record<string, unknown>)) {
    const info = REGISTRY.get(code);
    if (!info) {
      problems.push({
        message: `"check.allow": ${JSON.stringify(code)} is not a check code (sk explain --list shows them)`,
        key: code,
      });
      continue;
    }
    if (info.fatal) {
      problems.push({ message: `"check.allow": ${code} stops the check and cannot be allowed`, key: code });
      continue;
    }
    if (!Array.isArray(value) || value.some((v) => typeof v !== "string" || !v.trim())) {
      problems.push({
        message: `"check.allow.${code}" must be a list of package names or app-relative globs`,
        key: code,
      });
      continue;
    }
    allow.set(
      code,
      value.map((v: string) => v.trim()),
    );
  }
  return { allow, problems };
}

export class Suppressions {
  private readonly globs = new Map<string, Bun.Glob>();

  constructor(private readonly allow: Map<string, string[]>) {}

  matches(code: string, file: string, packageName: string | null): boolean {
    for (const pattern of this.allow.get(code) ?? []) {
      if (packageName !== null && pattern === packageName) return true;
      let glob = this.globs.get(pattern);
      if (!glob) this.globs.set(pattern, (glob = new Bun.Glob(pattern.replace(/^\.\//, ""))));
      if (glob.match(file)) return true;
    }
    return false;
  }
}
