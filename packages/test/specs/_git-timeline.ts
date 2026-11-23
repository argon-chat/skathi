// A small repository with several authors, branches, merges, tags and a remote, for the git template's
// history timeline.

import { spawnSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const AUTHORS = [
  ["Mira Okafor", "mira@example.com"],
  ["Jonas Berg", "jonas@example.com"],
  ["Lena Wu", "lena@example.com"],
  ["Tomas Reyes", "tomas@example.com"],
  ["Priya Nair", "priya@example.com"],
] as const;

export function seedTimeline(dir: string, remote: string, env: Record<string, string>) {
  mkdirSync(join(dir, "notes"), { recursive: true });
  let clock = Math.floor(Date.now() / 1000) - 40 * 3600 * 6;
  let file = 0;
  const run = (args: string[], extra: Record<string, string> = {}) => {
    const r = spawnSync("git", args, {
      cwd: dir,
      encoding: "utf8",
      env: { ...process.env, ...env, GIT_OPTIONAL_LOCKS: "0", LC_ALL: "C", ...extra },
    });
    if (r.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${r.stderr}`);
    return r.stdout;
  };
  const stamp = (who: number) => {
    clock += 3600 * (3 + (file % 5));
    const [name, email] = AUTHORS[who % AUTHORS.length]!;
    const date = `${clock} +0000`;
    return {
      GIT_AUTHOR_NAME: name,
      GIT_AUTHOR_EMAIL: email,
      GIT_COMMITTER_NAME: name,
      GIT_COMMITTER_EMAIL: email,
      GIT_AUTHOR_DATE: date,
      GIT_COMMITTER_DATE: date,
    };
  };
  const commit = (who: number, subject: string, body = "") => {
    file++;
    writeFileSync(join(dir, `notes/${file}.md`), `# ${subject}\n\nrevision ${file}\n`);
    run(["add", "-A"]);
    run(["commit", "-q", "-m", body ? `${subject}\n\n${body}` : subject], stamp(who));
  };
  const merge = (who: number, branch: string, subject: string) => {
    run(["merge", "-q", "--no-ff", "-m", subject, branch], stamp(who));
  };

  run(["init", "-q", "-b", "main"]);
  run(["config", "core.autocrlf", "false"]);
  run(["config", "commit.gpgsign", "false"]);
  run(["config", "tag.gpgsign", "false"]);
  commit(0, "Initial commit");
  commit(0, "Add the project layout", "Source, tests and docs folders with a first README.");
  commit(1, "Set up continuous integration");
  run(["tag", "v0.1.0"]);

  run(["switch", "-q", "-c", "feature/search"]);
  commit(2, "Search: index files on startup", "The index is built in the background and kept on disk.");
  commit(2, "Search: rank results by recency");
  run(["switch", "-q", "main"]);
  commit(1, "Fix a crash when the config file is empty");
  run(["switch", "-q", "-c", "fix/login-timeout"]);
  commit(4, "Retry the login request once on timeout", "Servers behind slow proxies dropped the first request.");
  run(["switch", "-q", "feature/search"]);
  commit(2, "Search: highlight matches in results");
  run(["switch", "-q", "main"]);
  commit(3, "Update dependencies");
  commit(4, "Raise the login timeout to 30 seconds");
  merge(1, "fix/login-timeout", "Merge branch 'fix/login-timeout'");
  commit(0, "Tune the startup sequence", "Defer loading plugins until the first window is shown.");
  merge(0, "feature/search", "Merge branch 'feature/search'");
  run(["tag", "v0.2.0"]);
  run(["branch", "-D", "fix/login-timeout"]);

  run(["switch", "-q", "-c", "feature/themes"]);
  commit(2, "Themes: read colours from the system");
  commit(0, "Themes: add a high-contrast palette");
  run(["switch", "-q", "main"]);
  commit(3, "Document the plugin API", "Covers loading, permissions and the event bus.");
  run(["switch", "-q", "-c", "release/0.3"]);
  commit(1, "Bump the version to 0.3.0");
  run(["switch", "-q", "feature/themes"]);
  commit(2, "Themes: animate the switch between palettes");
  run(["switch", "-q", "main"]);
  commit(4, "Show the sync state in the status bar");

  spawnSync("git", ["init", "-q", "--bare", remote], { env: { ...process.env, ...env } });
  run(["remote", "add", "origin", remote]);
  run(["push", "-q", "origin", "main~1:refs/heads/main", "feature/themes", "release/0.3"]);
  run(["fetch", "-q", "origin"]);
  run(["branch", "-q", "--set-upstream-to=origin/main", "main"]);
  commit(1, "Polish the welcome screen");
}
