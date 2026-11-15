import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/** What `bun prod.ts --graph <file>` writes: the files whose code reaches the bundle. */
export interface ModuleGraph {
  schema: "skathi.module-graph";
  version: 1;
  entry: string;
  appRoot: string;
  /** Bare module names the build maps to a file (vue, the runtime, sk.config.json "aliases"). */
  aliases: Record<string, string>;
  /** Ambient declarations the runtime contributes. */
  declarations: string[];
  /** Absolute paths of the .js/.ts/.vue/.svelte files with code in the bundle. */
  modules: string[];
  errors: GraphError[];
}

export interface GraphError {
  message: string;
  file: string | null;
  /** One-based. */
  line: number | null;
  /** Zero-based. */
  column: number | null;
  specifier: string | null;
}

export class GraphFailure extends Error {}

export async function buildGraph(options: {
  prodTs: string;
  entry: string;
  appRoot: string;
  configPath: string | null;
}): Promise<ModuleGraph> {
  const work = mkdtempSync(join(tmpdir(), "sk-check-"));
  try {
    const out = join(work, "graph.json");
    const child = Bun.spawn(
      [
        process.execPath,
        options.prodTs,
        options.entry,
        join(work, "app.prod.js"),
        `--app-root=${options.appRoot}`,
        ...(options.configPath ? [`--config=${options.configPath}`] : []),
        `--graph=${out}`,
      ],
      { cwd: join(options.prodTs, ".."), stdout: "pipe", stderr: "pipe" },
    );
    const [stdout, stderr, status] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ]);
    if (status !== 0 || !existsSync(out)) {
      const output = `${stderr}${stdout}`.trim();
      throw new GraphFailure(output || `the production build exited with ${child.signalCode ?? status}`);
    }
    const graph = JSON.parse(readFileSync(out, "utf8")) as ModuleGraph;
    if (graph.schema !== "skathi.module-graph" || graph.version !== 1)
      throw new GraphFailure("the production builder wrote a module graph this sk check does not understand");
    return graph;
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}
