// `sk create --template git`: the git client opens a temporary repository, lists its changes, stages a
// file, a hunk and a single line through the diff, commits, shows the commit in the history and switches
// branches. Every step is checked against git itself.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { create } from "../../cli/cli/create";
import { compileAppEffects, writeEffectFiles } from "../../cli/effects/compile";
import { resolveCompiler } from "../../cli/effects/redslangc";
import { appDataDir, expect, launch, REPO, type SkApp, WORK_DIR } from "../src/index";
import { seedTimeline } from "./_git-timeline";

const T = 60_000;
const hasGit = spawnSync("git", ["--version"], { encoding: "utf8" }).status === 0;
if (!hasGit) console.warn("[git-template.spec] skipped: git is not installed or not on PATH");

const dir = join(WORK_DIR, `git-template-${process.pid}`);
// Outside the checkout, and fenced by GIT_CEILING_DIRECTORIES, so no git call can reach an enclosing repository.
const scratch = hasGit ? mkdtempSync(join(tmpdir(), "sk-git-template-")) : "";
const ceiling = { GIT_CEILING_DIRECTORIES: scratch };
const repoDir = join(scratch, "repo");
const plainDir = join(scratch, "plain");
const remoteDir = join(scratch, "remote.git");
const profile = `sk-test-git-template-${process.pid}`;
const shots = process.env.SK_SHOTS;

function git(...args: string[]): string {
  const r = spawnSync("git", args, {
    cwd: repoDir,
    encoding: "utf8",
    env: { ...process.env, ...ceiling, GIT_OPTIONAL_LOCKS: "0", LC_ALL: "C" },
  });
  if (r.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${r.stderr}`);
  return r.stdout;
}

const lines = (n: number, tag: string) => Array.from({ length: n }, (_, i) => `const ${tag}${i + 1} = ${i + 1};`);

function seedRepo() {
  mkdirSync(join(repoDir, "src"), { recursive: true });
  mkdirSync(plainDir, { recursive: true });
  git("init", "-q", "-b", "main");
  git("config", "user.name", "Test Author");
  git("config", "user.email", "author@example.com");
  git("config", "core.autocrlf", "false");
  git("config", "commit.gpgsign", "false");
  writeFileSync(join(repoDir, "README.md"), "# Sample\n\nA repository for the git template test.\n");
  writeFileSync(join(repoDir, "src", "app.ts"), lines(40, "value").join("\n") + "\n");
  writeFileSync(join(repoDir, "old-name.txt"), "this file gets renamed\nand keeps its text\n");
  git("add", "-A");
  git("commit", "-q", "-m", "Initial commit");
  writeFileSync(join(repoDir, "notes.txt"), "first note\n");
  git("add", "notes.txt");
  git("commit", "-q", "-m", "Add notes");
  git("branch", "feature/login");
  spawnSync("git", ["init", "-q", "--bare", remoteDir], {
    env: { ...process.env, ...ceiling },
  });
  git("remote", "add", "origin", remoteDir);

  const app = lines(40, "value");
  app[2] = "const value3 = 300;";
  app[3] = "const value4 = 400;";
  app[32] = "const value33 = 3300;";
  writeFileSync(join(repoDir, "src", "app.ts"), app.join("\n") + "\n");
  writeFileSync(join(repoDir, "new.txt"), "brand new\n");
  git("mv", "old-name.txt", "renamed.txt");
}

let app: SkApp | undefined;
if (hasGit) {
  seedRepo();
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(WORK_DIR, { recursive: true });
  create(dir, {
    name: "git-template",
    template: "git",
    platforms: ["windows"],
  });
  const entry = relative(REPO, join(dir, "src/main.ts")).split("\\").join("/");
  // Without the shader compiler the app runs with its effects missing, which changes nothing a test checks.
  const compiler = (await resolveCompiler().catch(() => null))?.path ?? null;
  const effects: Record<string, string> = {};
  if (compiler) {
    effects.SK_EFFECTS_DIR = join(dir, "effects");
    writeEffectFiles(effects.SK_EFFECTS_DIR, (await compileAppEffects(dir, compiler)).files);
  }
  app = await launch({
    app: entry,
    window: [1280, 820],
    label: "git-template",
    args: [repoDir],
    env: { SK_APP_NAME: profile, ...ceiling, ...effects },
  });
}

afterAll(async () => {
  await app?.close();
  rmSync(dir, { recursive: true, force: true });
  if (scratch) rmSync(scratch, { recursive: true, force: true });
  const deadline = Date.now() + 5_000;
  for (;;) {
    try {
      rmSync(appDataDir(profile), { recursive: true, force: true });
      break;
    } catch (error) {
      if (Date.now() >= deadline) throw error;
      await Bun.sleep(100);
    }
  }
}, 30_000);

async function until<V>(read: () => V | Promise<V>, done: (value: V) => boolean, what: string): Promise<V> {
  const deadline = Date.now() + 10_000;
  for (;;) {
    const value = await read();
    if (done(value)) return value;
    if (Date.now() >= deadline) throw new Error(`${what}: ${JSON.stringify(value)}\n${await app!.tailLogs()}`);
    await Bun.sleep(80);
  }
}

const paths = (list: string) =>
  app!
    .eval(`JSON.stringify([...document.querySelectorAll("#${list} .file")].map((e) => e.getAttribute("data-path")))`)
    .then((s: string) => (JSON.parse(s) as string[]).sort());

async function theme(t: "light" | "dark") {
  await app!.eval(`__SK_WINDOW__.appWindow.theme = ${JSON.stringify(t)}; "ok"`);
  await Bun.sleep(300);
}

/** With SK_SHOTS set, saves `<name>-dark.png` and `<name>-light.png`, and leaves the app dark. */
async function shot(name: string) {
  if (!shots) return;
  for (const t of ["dark", "light"] as const) {
    await theme(t);
    await Bun.sleep(250);
    await app!.screenshot(join(shots, `${name}-${t}.png`));
  }
  await theme("dark");
}

/** A view transition's overlay takes the pointer while it runs, so wait it out before a real click. */
async function settled() {
  await until(
    () => app!.eval(`!document.activeViewTransition`),
    (idle: boolean) => idle,
    "view transition finished",
  );
}

async function tab(name: "changes" | "history") {
  await app!.locator(`#tab-${name}`).click();
  await settled();
}

const notices = () =>
  app!
    .eval(`JSON.stringify([...document.querySelectorAll(".notice.ok b")].map((e) => e.textContent.trim()))`)
    .then((s: string) => JSON.parse(s) as string[]);

async function selectFile(list: "unstaged" | "staged", path: string) {
  await settled();
  await app!.locator(`#${list} .file[data-path="${path}"]`).click({ real: true });
  await expect(app!.locator(".diff-head .path b")).toHaveText(path.split("/").pop()!);
}

const cached = (path: string) => git("diff", "--cached", "--no-color", "--", path);

/** Clicks the gutter of the diff line whose text is `text`, as a person picks a line. */
async function pickLine(text: string) {
  const at = await until(
    () =>
      app!.eval(`(() => {
        for (const ed of document.querySelectorAll("#diff-canvas sk-editor")) {
          for (let i = 0; i < ed.lineCount; i++) {
            const line = ed.line(i);
            if (line.text !== ${JSON.stringify(text)}) continue;
            const row = ed.rectsFor(line.from, line.to)[0];
            const picker = ed.parentElement.querySelector(".picker").getBoundingClientRect();
            return JSON.stringify({ x: picker.x + picker.width / 2, y: row.y + row.height / 2 });
          }
        }
        return null;
      })()`),
    (s: string | null) => s !== null,
    `diff line ${text}`,
  );
  const { x, y } = JSON.parse(at!) as { x: number; y: number };
  app!.notePointer({ kind: "point", x, y });
  await app!.send("mouse", { x, y, kind: "click" });
}

describe.skipIf(!hasGit)("the git template", () => {
  test(
    "lists the unstaged and staged changes with their kinds",
    async () => {
      await expect(app!.locator("#unstaged .file")).toHaveCount(2, {
        timeout: 30_000,
      });
      bunExpect(await paths("unstaged")).toEqual(["new.txt", "src/app.ts"]);
      bunExpect(await paths("staged")).toEqual(["renamed.txt"]);
      await expect(app!.locator('#unstaged .file[data-path="new.txt"] .badge')).toHaveText("U");
      await expect(app!.locator('#staged .file[data-path="renamed.txt"] .badge')).toHaveText("R");
      await expect(app!.locator("#branch-button .crumb-label")).toHaveText("main");
    },
    T,
  );

  test(
    "shows the diff with both hunks and word highlights, dark and light",
    async () => {
      await selectFile("unstaged", "src/app.ts");
      await expect(app!.locator(".hunk")).toHaveCount(2);
      await shot("changes");
    },
    T,
  );

  test(
    "stages a whole file with the keyboard",
    async () => {
      await selectFile("unstaged", "new.txt");
      await app!.locator("#unstaged").press("Space");
      await until(
        () => paths("staged"),
        (p) => p.includes("new.txt"),
        "new.txt staged",
      );
      bunExpect(git("diff", "--cached", "--name-only").split("\n")).toContain("new.txt");
    },
    T,
  );

  test(
    "stages one hunk of a file",
    async () => {
      await selectFile("unstaged", "src/app.ts");
      await app!.locator(".hunk-apply").first().click();
      await until(
        () => cached("src/app.ts"),
        (d) => d.includes("+const value3 = 300;"),
        "first hunk staged",
      );
      const diff = cached("src/app.ts");
      bunExpect(diff).toContain("+const value4 = 400;");
      bunExpect(diff).not.toContain("value33 = 3300");
      await until(
        () => paths("staged"),
        (p) => p.includes("src/app.ts"),
        "src/app.ts in staged list",
      );
    },
    T,
  );

  test(
    "stages a single picked line",
    async () => {
      await selectFile("unstaged", "src/app.ts");
      await expect(app!.locator(".hunk")).toHaveCount(1);
      await pickLine("const value33 = 3300;");
      await expect(app!.locator(".picked-count")).toHaveText("1 line");
      await shot("line-selection");
      await app!.locator("#apply-lines").click();
      await until(
        () => cached("src/app.ts"),
        (d) => d.includes("+const value33 = 3300;"),
        "picked line staged",
      );
      bunExpect(cached("src/app.ts")).not.toContain("-const value33 = 33;");
    },
    T,
  );

  test(
    "unstages a single picked line of the staged diff",
    async () => {
      await selectFile("staged", "src/app.ts");
      await pickLine("const value4 = 400;");
      await expect(app!.locator("#apply-lines")).toHaveText("Unstage lines");
      await app!.locator("#apply-lines").click();
      await until(
        () => cached("src/app.ts"),
        (d) => !d.includes("+const value4 = 400;"),
        "picked line unstaged",
      );
      const diff = cached("src/app.ts");
      bunExpect(diff).toContain("+const value3 = 300;");
      bunExpect(diff).toContain("+const value33 = 3300;");
    },
    T,
  );

  test(
    "discards and unstages a deleted line of a changed run in its place",
    async () => {
      writeFileSync(join(repoDir, "swap.txt"), "a\nb\nc\n");
      writeFileSync(join(repoDir, "swap2.txt"), "p\nq\nr\n");
      git("add", "swap.txt", "swap2.txt");
      git("commit", "-q", "-m", "Add the swap files", "--", "swap.txt", "swap2.txt");
      writeFileSync(join(repoDir, "swap.txt"), "a\nX\nc\n");
      writeFileSync(join(repoDir, "swap2.txt"), "p\nY\nr\n");
      git("add", "swap2.txt");
      await app!.locator("#refresh-button").click();

      await selectFile("unstaged", "swap.txt");
      await pickLine("b");
      await app!.locator("#discard-lines").click();
      await app!.locator("#confirm-ok").click();
      await until(
        () => readFileSync(join(repoDir, "swap.txt"), "utf8"),
        (t) => t !== "a\nX\nc\n",
        "picked deletion discarded",
      );
      bunExpect(readFileSync(join(repoDir, "swap.txt"), "utf8")).toBe("a\nb\nX\nc\n");

      await selectFile("staged", "swap2.txt");
      await pickLine("q");
      await app!.locator("#apply-lines").click();
      await until(
        () => git("show", ":swap2.txt"),
        (t) => t !== "p\nY\nr\n",
        "picked deletion unstaged",
      );
      bunExpect(git("show", ":swap2.txt")).toBe("p\nq\nY\nr\n");

      git("reset", "-q", "--", "swap2.txt");
      git("checkout", "--", "swap.txt", "swap2.txt");
      await app!.locator("#refresh-button").click();
      await until(paths.bind(null, "unstaged"), (p) => !p.includes("swap.txt"), "swap files clean");
    },
    T,
  );

  test(
    "commits the staged changes, and shows a failing hook's stderr when amending",
    async () => {
      await app!.locator("#message").type("Rework values");
      await shot("commit-editor");
      await app!.locator("#commit-button").click();
      await until(
        () => git("log", "-1", "--format=%s").trim(),
        (s) => s === "Rework values",
        "commit made",
      );
      bunExpect(git("diff", "--cached", "--name-only").trim()).toBe("");
      const short = git("rev-parse", "--short", "HEAD").trim();
      await until(notices, (n) => n.some((t) => t.includes(short)), "commit notice with the short hash");
      await shot("commit-landed");

      await app!.locator("#amend").click();
      await until(
        () => app!.eval(`document.querySelector("#message").value.text`),
        (t: string) => t === "Rework values",
        "amend loads the last message",
      );
      writeFileSync(
        join(repoDir, ".git", "hooks", "pre-commit"),
        "#!/bin/sh\necho 'pre-commit hook: lint failed' >&2\nexit 1\n",
        { mode: 0o755 },
      );
      await app!.locator("#commit-button").click();
      await expect(app!.locator("#commit-error pre")).toContainText("lint failed");
      await shot("commit-error");
      rmSync(join(repoDir, ".git", "hooks", "pre-commit"));
      await app!.locator("#amend").click();
    },
    T,
  );

  test(
    "shows the new commit at the top of the history with its files",
    async () => {
      await tab("history");
      await expect(app!.locator("#history .commit .subject").first()).toHaveText("Rework values");
      await expect(app!.locator("#history .commit .ref.head").first()).toHaveText("main");
      await expect(app!.locator("#commit-header .subject")).toHaveText("Rework values");
      await expect(app!.locator(".split .file")).toHaveCount(3);
      await shot("history");
    },
    T,
  );

  test(
    "discards the rest after a confirmation, then switches branch from the branch menu",
    async () => {
      await tab("changes");
      await selectFile("unstaged", "src/app.ts");
      await app!.locator("#discard-file").click();
      await expect(app!.locator("#confirm-ok")).toHaveText("Discard");
      await app!.locator("#confirm-ok").click();
      await until(
        () => git("status", "--porcelain").trim(),
        (s) => s === "",
        "working tree clean",
      );

      await tab("history");
      await app!.locator("#branch-button").click();
      await expect(app!.locator('#branch-menu .item[data-branch="feature/login"]')).toHaveCount(1);
      await shot("branch-switcher");
      await app!.locator('#branch-menu .item[data-branch="feature/login"]').click();
      await until(
        () => git("branch", "--show-current").trim(),
        (b) => b === "feature/login",
        "branch switched",
      );
      await expect(app!.locator("#branch-button .crumb-label")).toHaveText("feature/login");
      await expect(app!.locator("#history .commit.is-head .subject")).toHaveText("Add notes");
      await expect(app!.locator("#history .commit.is-head .ref.head")).toHaveText("feature/login");
    },
    T,
  );

  test(
    "creates a branch by typing its name",
    async () => {
      await app!.locator("#branch-button").click();
      await app!.locator("#branch-filter").type("topic/new-work");
      await app!.locator("#create-branch").click();
      await until(
        () => git("branch", "--show-current").trim(),
        (b) => b === "topic/new-work",
        "branch created",
      );
      await expect(app!.locator("#branch-button .crumb-label")).toHaveText("topic/new-work");
    },
    T,
  );

  test(
    "pushes a branch without an upstream to origin and sets it",
    async () => {
      await app!.locator("#push-button").click();
      await until(
        () =>
          spawnSync("git", ["--git-dir", remoteDir, "branch", "--list", "topic/new-work"], { encoding: "utf8" }).stdout,
        (out) => out.includes("topic/new-work"),
        "branch pushed",
      );
      await until(
        // push writes the remote ref before the upstream config, so "no upstream" is not a failure yet.
        () => git("config", "--default", "", "--get", "branch.topic/new-work.merge").trim(),
        (m) => m === "refs/heads/topic/new-work",
        "upstream set",
      );
      await until(notices, (n) => n.includes("Push finished"), "push notice");
    },
    T,
  );

  test(
    "says so when a folder is not a repository, and reopens a recent one",
    async () => {
      await tab("changes");
      await expect(app!.locator("#clean")).toHaveCount(1);
      await shot("empty-clean");
      await app!.locator("#repo-button").click();
      await app!.locator("#close-repo").click();
      await expect(app!.locator("#open-button")).toHaveCount(1);
      await app!.locator("#open-button").click();
      const dialog = await app!.waitForDialog();
      await app!.answerDialog(dialog.id, plainDir);
      await expect(app!.locator("#open-error b")).toHaveText("This folder is not a git repository.");
      await shot("not-a-repository");
      await app!.locator(".recent-open").first().click();
      await expect(app!.locator("#branch-button .crumb-label")).toHaveText("topic/new-work");
    },
    T,
  );

  test(
    "draws branches, merges and tags on the history timeline",
    async () => {
      const timelineDir = join(scratch, "timeline");
      seedTimeline(timelineDir, join(scratch, "timeline-origin.git"), ceiling);
      await app!.locator("#repo-button").click();
      await app!.locator("#close-repo").click();
      await app!.locator("#open-button").click();
      const dialog = await app!.waitForDialog();
      await app!.answerDialog(dialog.id, timelineDir);
      await expect(app!.locator("#branch-button .crumb-label")).toHaveText("main");
      await tab("history");
      await expect(app!.locator("#history .commit .subject").first()).toHaveText("Polish the welcome screen");
      await expect(app!.locator("#history .commit.merge")).toHaveCount(2);
      await expect(app!.locator("#history .ref.tag")).toHaveCount(2);
      const lanes = await app!.eval(
        `new Set([...document.querySelectorAll("#history .commit")].map((e) => e.style.getPropertyValue("--node-x"))).size`,
      );
      bunExpect(lanes).toBeGreaterThanOrEqual(3);
      await app!.locator(".thead").hover();
      await shot("history-timeline");
      await app!.locator("#history .commit.merge").first().click();
      await expect(app!.locator("#commit-header .subject")).toHaveText("Merge branch 'feature/search'");
      await app!.locator(".thead").hover();
      await shot("history-detail");
      await app!.locator("#repo-button").click();
      await app!.locator("#close-repo").click();
      await expect(app!.locator(".recent-open")).toHaveCount(2);
      await shot("welcome");
    },
    T,
  );

  test(
    "reads non-ASCII commit messages that the log's chunks split",
    async () => {
      // ~480 KB of subjects in 3- and 4-byte UTF-8, so the pipe's chunk edges fall inside characters.
      const unicodeDir = join(scratch, "unicode");
      mkdirSync(unicodeDir, { recursive: true });
      spawnSync("git", ["init", "-q", "-b", "main", unicodeDir], {
        env: { ...process.env, ...ceiling },
      });
      const stream: string[] = [];
      for (let i = 1; i <= 150; i++) {
        const msg = `${i}: ${"日本語のテキスト🙂Жж".repeat(100)}\n`;
        stream.push(
          "commit refs/heads/main",
          `mark :${i}`,
          `committer Ünïcødé <u@example.com> ${1_700_000_000 + i * 60} +0000`,
          `data ${Buffer.byteLength(msg)}`,
          msg,
          ...(i > 1 ? [`from :${i - 1}`] : []),
          `M 644 inline n${i % 7}.txt`,
          `data ${String(i).length + 1}`,
          `${i}`,
          "",
        );
      }
      const imported = spawnSync("git", ["fast-import", "--quiet"], {
        cwd: unicodeDir,
        input: stream.join("\n") + "\n",
        env: { ...process.env, ...ceiling },
      });
      bunExpect(imported.status).toBe(0);
      spawnSync("git", ["checkout", "-q", "-f", "main"], {
        cwd: unicodeDir,
        env: { ...process.env, ...ceiling },
      });
      const expected = new Map(
        spawnSync("git", ["log", "--format=%H%x00%s"], {
          cwd: unicodeDir,
          encoding: "utf8",
        })
          .stdout.trim()
          .split("\n")
          .map((l) => l.split("\0") as [string, string]),
      );
      bunExpect(expected.size).toBe(150);

      await app!.locator("#open-button").click();
      const dialog = await app!.waitForDialog();
      await app!.answerDialog(dialog.id, unicodeDir);
      await tab("history");
      await until(
        () => app!.eval(`document.querySelectorAll("#history .commit:not(.pending)").length`),
        (n: number) => n > 0,
        "history rows",
      );
      const seen = new Map<string, string>();
      for (let top = 0; seen.size < expected.size && top < 150 * 34; top += 500) {
        const rows = await until(
          () =>
            app!.eval(`(() => {
              const el = document.querySelector("#history");
              el.scrollTop = ${top};
              if (document.querySelector("#history .commit.pending")) return null;
              return JSON.stringify([...document.querySelectorAll("#history .commit")].map((e) => [e.dataset.sha, e.querySelector(".subject").textContent]));
            })()`),
          (s: string | null) => s !== null,
          `history rows at ${top}`,
        );
        for (const [sha, subject] of JSON.parse(rows!) as [string, string][]) seen.set(sha, subject);
      }
      bunExpect(seen.size).toBe(expected.size);
      for (const [sha, subject] of seen) bunExpect(subject).toBe(expected.get(sha)!);
    },
    T,
  );
});
