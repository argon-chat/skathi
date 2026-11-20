import { readFileSync, statSync } from "node:fs";
import { dirname, isAbsolute } from "node:path";
import type { Plugin } from "vite";
import type { AotInputs } from "../aot-inputs";
import { sfcCompiler } from "../runtime-path";

/** Archive on-disk typed inputs alongside the code produced by application plugins. */
export function viteInventory(inputs: AotInputs): Plugin {
  return {
    name: "skathi:aot-inputs",
    enforce: "post",
    transform(code, id) {
      if (!isAbsolute(id) || !/\.(?:vue|svelte|[cm]?[jt]sx?)$/i.test(id) || id.includes("?") || id.includes("\0"))
        return;
      if (!statSync(id, { throwIfNoEntry: false })?.isFile()) return;
      const original = readFileSync(id, "utf8");
      if (id.endsWith(".vue")) {
        const { descriptor } = sfcCompiler(dirname(id)).parse(original, { filename: id });
        const scripts = [descriptor.script, descriptor.scriptSetup];
        const loader = scripts.some((block) => block?.lang === "ts" || block?.lang === "tsx") ? "ts" : "js";
        inputs.vue(id, original, code, loader, scripts.map((block) => block?.content ?? "").join("\n"));
      } else if (/\.svelte(?:\.[jt]s)?$/i.test(id)) {
        inputs.svelte(id, original, code);
      } else {
        inputs.source(id, original, code, original);
      }
    },
  };
}
