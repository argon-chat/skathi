import type { NodeId, TreeNode } from "./protocol";

function rowText(node: TreeNode): string {
  const classes = (node.classes || []).join(" ");
  const id = node.props?.id || "";
  return `${node.tag || ""} ${classes} ${id} ${node.text || ""}`.toLowerCase();
}

/**
 * The Elements search: a node the target's selector engine matched (`.class`, `#id`, `a > b`), or
 * one whose tag, classes, id or text contain the query. `selectorHits` is null when the query is
 * not a valid selector.
 */
export function searchHit(node: TreeNode, query: string, selectorHits: ReadonlySet<NodeId> | null): boolean {
  const text = query.trim().toLowerCase();
  if (!text) return false;
  return !!selectorHits?.has(node.id) || rowText(node).includes(text);
}
