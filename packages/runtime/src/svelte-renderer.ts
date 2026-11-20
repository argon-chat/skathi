import { createRenderer } from "svelte/renderer";
import { svelteNodeOps } from "@skathi-native/runtime";
import type { MirrorElement, MirrorNode } from "./runtime/types";

class Fragment {
  children: MirrorNode[] = [];
}

const { nodeOps, detachNode, looseCreated, patchProp } = svelteNodeOps;

// A template fragment exists only while Svelte assembles its children. It has no host node.
const fragments = new WeakMap<MirrorNode, Fragment>();

function domContainer(other: object | null) {
  if (other !== null) throw new Error("Skathi can embed the Svelte DOM renderer only.");
  const node = looseCreated(nodeOps.createElement("sk-svelte-host"));
  return { container: node, node };
}

function removeFromFragment(node: MirrorNode): void {
  const fragment = fragments.get(node);
  if (fragment) {
    fragment.children.splice(fragment.children.indexOf(node), 1);
    fragments.delete(node);
  }
}

function inheritNamespace(node: MirrorNode, namespace: "svg" | "mathml"): void {
  if (node.kind !== 1 || node._ns) return;
  node._ns = namespace;
  node.tagName = node.tag;
  if (node.tag !== "foreignObject") for (const child of node.childNodes) inheritNamespace(child, namespace);
}

const renderer = createRenderer<{
  fragment: Fragment;
  element: MirrorElement;
  text: MirrorNode;
  comment: MirrorNode;
}>({
  createFragment: () => new Fragment(),
  createElement: (name) =>
    looseCreated(nodeOps.createElement(name, name === "svg" ? "svg" : name === "math" ? "mathml" : undefined)),
  createText: (data) => looseCreated(nodeOps.createText(data)),
  createComment(data) {
    const node = looseCreated(nodeOps.createComment(data));
    node.text = data;
    return node;
  },
  getNodeType: (node) =>
    node instanceof Fragment ? "fragment" : node.kind === 1 ? "element" : node.kind === 2 ? "text" : "comment",
  getText: (node) => node.text ?? "",
  setText: nodeOps.setText,
  getAttribute: (el, name) => el.getAttribute(name),
  hasAttribute: (el, name) => el.hasAttribute(name),
  setAttribute: (el, name, value: unknown) => patchProp(el, name, el.getAttribute(name), value, el._ns),
  removeAttribute: (el, name) => patchProp(el, name, el.getAttribute(name), null, el._ns),
  getFirstChild: (node) => (node instanceof Fragment ? node.children[0] : node.firstChild) ?? null,
  getLastChild: (node) => (node instanceof Fragment ? node.children.at(-1) : node.lastChild) ?? null,
  getParent: (node) => fragments.get(node) ?? node.parentNode,
  getNextSibling(node) {
    const children = fragments.get(node)?.children;
    return children ? (children[children.indexOf(node) + 1] ?? null) : nodeOps.nextSibling(node);
  },
  insert(parent, node, anchor) {
    if (node === anchor) return;
    const nodes = node instanceof Fragment ? node.children.splice(0) : [node];
    for (const child of nodes) {
      if (node instanceof Fragment) fragments.delete(child);
      else removeFromFragment(child);
      if (parent instanceof Fragment) {
        detachNode(child);
        parent.children.splice(anchor ? parent.children.indexOf(anchor) : parent.children.length, 0, child);
        fragments.set(child, parent);
      } else {
        if (parent._ns && parent.tag !== "foreignObject") inheritNamespace(child, parent._ns);
        nodeOps.insert(child, parent, anchor);
      }
    }
  },
  remove(node) {
    removeFromFragment(node);
    detachNode(node);
  },
  addEventListener: (el, name, fn, options) => el.addEventListener(name, fn, options),
  removeEventListener: (el, name, fn, options) => el.removeEventListener(name, fn, options),
  embed: ({ host }) => domContainer(host),
  host: ({ guest }) => domContainer(guest),
});

export default renderer;
