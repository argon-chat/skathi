import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { HastPluginDefinition, MdastPluginDefinition } from "satteri";
import { isNew, newLabel } from "./release";

const contentRoot = fileURLToPath(new URL("../../", import.meta.url));

function resolveDocLink(url: string, file: URL | undefined): string {
  if (!file || /^(?:[a-z][a-z\d+.-]*:|\/|#)/i.test(url)) return url;
  const match = /^([^?#]+)\.(?:md|mdx)(#.*)?$/.exec(url);
  if (!match?.[1]) return url;
  const path = relative(contentRoot, resolve(dirname(fileURLToPath(file)), match[1]))
    .replaceAll("\\", "/")
    .replace(/^pages\//, "");
  return path.startsWith("..") ? url : `/${path}/${match[2] ?? ""}`;
}

const docsContent: MdastPluginDefinition = {
  name: "docs-content",
  heading(node, context) {
    if (node.depth === 1) context.removeNode(node);
  },
  link(node, context) {
    const url = resolveDocLink(node.url, context.fileURL);
    if (url !== node.url) context.setField(node, "url", url);
  },
  definition(node, context) {
    const url = resolveDocLink(node.url, context.fileURL);
    if (url !== node.url) context.setField(node, "url", url);
  },
};

export default docsContent;

const sinceMark = /\s*\(since (\d+\.\d+)\)\s*$/;

/** Strips `(since 0.4)` from a heading, badges it while 0.4 is current, and records it by text for the TOC. */
export const sectionSince: HastPluginDefinition = {
  name: "section-since",
  element: {
    filter: ["h2", "h3", "h4"],
    visit(node, context) {
      const index = node.children.length - 1;
      const last = node.children[index];
      if (last?.type !== "text") return;
      const since = sinceMark.exec(last.value)?.[1];
      if (!since) return;
      const astro = context.data["astro"] as { frontmatter?: Record<string, unknown> } | undefined;
      if (astro?.frontmatter) {
        const marks = (astro.frontmatter["sectionSince"] ??= {}) as Record<string, string>;
        marks[context.textContent(node).replace(sinceMark, "")] = since;
      }
      context.removeChildAt(node, index);
      context.appendChild(node, { type: "text", value: last.value.replace(sinceMark, "") });
      if (isNew(since))
        context.appendChild(node, {
          type: "element",
          tagName: "span",
          properties: { className: ["new-badge"], title: newLabel },
          children: [],
        });
    },
  },
};
