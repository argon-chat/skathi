import { globSync } from "node:fs";
import { join, relative } from "node:path";
import { normalizePath, type Plugin } from "vite";
import { workspaceRoot } from "./workspace";

export function watchApiSources(): Plugin {
  return {
    name: "skathi-api-sources",
    apply: "serve",
    configureServer(server) {
      const sources = [
        ...globSync("crates/sk-*/src", { cwd: workspaceRoot }),
        "vendor/redjs/redjs/src/webapi",
        "packages/runtime/src",
        "packages/runtime/env.d.ts",
        "packages/runtime/tsconfig.app.json",
        "tsconfig.base.json",
      ];
      server.watcher.add(sources.map((source) => join(workspaceRoot, source)));
    },
    hotUpdate({ file, modules }) {
      const path = normalizePath(relative(workspaceRoot, file));
      let extractor: string;
      if (/^packages\/runtime\/(?:.*\.ts|tsconfig\.app\.json)$/.test(path) || path === "tsconfig.base.json") {
        extractor = "bindings-reference.ts";
      } else if (/^(?:crates\/sk-[^/]+\/src|vendor\/redjs\/redjs\/src\/webapi)\/.+\.rs$/.test(path)) {
        extractor = "rust-bindings.ts";
      } else return;
      const generators = this.environment.moduleGraph.getModulesByFile(
        normalizePath(join(workspaceRoot, "docs/scripts", extractor)),
      );
      return generators?.size ? [...new Set([...modules, ...generators])] : undefined;
    },
  };
}
