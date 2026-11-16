import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { MdastPluginDefinition } from "satteri";

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
