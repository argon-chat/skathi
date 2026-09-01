// How the inspector's Elements tree NAMES a row.
//
// The bug this pins: the retained tree gives text nodes and Vue's fragment
// anchors `tag: 0` ("no tag"), and the host's name table used to hand id 0 to
// the first name anybody interned — `create_root`'s `"root"`. So every anchor
// came back as `tag: "root"` and the panel, which decided a row's shape from
// its tag, drew runs of `<root>` rows between the real elements.
//
// Two halves, and this file covers the second:
//   * the host reserves name 0 (crates/vn-tree/src/tree.rs; the Rust test is
//     `text_and_anchor_nodes_have_no_tag_name` in vn-tree's devtools.rs);
//   * the panel decides by KIND and never by tag — which is what keeps it
//     honest against a host that predates the fix, and is what is asserted
//     here.

import { describe, expect, test } from "bun:test";
import { nodeLabel, rowKind, rowVisible } from "../../../apps/devtools/src/dump";
import type { TreeNode } from "../../../apps/devtools/src/protocol";

const node = (over: Partial<TreeNode>): TreeNode =>
  ({ id: 1, kind: "element", tag: "div", bounds: null, ...over }) as TreeNode;

describe("rowKind", () => {
  test("maps the three node kinds the wire sends", () => {
    expect(rowKind(node({ kind: "element" }))).toBe("element");
    expect(rowKind(node({ kind: "text", tag: "" }))).toBe("text");
    expect(rowKind(node({ kind: "anchor", tag: "" }))).toBe("anchor");
  });

  test("an unknown kind is drawn as an element rather than dropped", () => {
    // A host newer than this inspector must not be able to make rows vanish.
    expect(rowKind(node({ kind: "portal" }))).toBe("element");
    expect(rowKind(undefined)).toBe("element");
  });

  test("kind wins over tag, so a PRE-FIX host still labels anchors right", () => {
    // What the old binary sent for every anchor and every text node.
    expect(rowKind(node({ kind: "anchor", tag: "root" }))).toBe("anchor");
    expect(rowKind(node({ kind: "text", tag: "root" }))).toBe("text");
  });
});

describe("nodeLabel", () => {
  test("an element is its tag plus its classes", () => {
    expect(nodeLabel(node({ tag: "div", classes: ["card", "on"] }))).toBe("div.card.on");
    expect(nodeLabel(node({ tag: "header" }))).toBe("header");
  });

  test("a fragment anchor is a comment row, never `root`", () => {
    expect(nodeLabel(node({ kind: "anchor", tag: "" }))).toBe("<!---->");
    // The regression, stated as an assertion: the old wire shape, still right.
    expect(nodeLabel(node({ kind: "anchor", tag: "root" }))).toBe("<!---->");
  });

  test("a text node is #text", () => {
    expect(nodeLabel(node({ kind: "text", tag: "", text: "hello" }))).toBe("#text");
    expect(nodeLabel(node({ kind: "text", tag: "root", text: "hello" }))).toBe("#text");
  });

  test("the document root is a real element called root and keeps its name", () => {
    expect(nodeLabel(node({ kind: "element", tag: "root" }))).toBe("root");
  });

  test("no node at all is an em dash, not a crash", () => {
    expect(nodeLabel(null)).toBe("—");
    expect(nodeLabel(undefined)).toBe("—");
  });
});

describe("rowVisible", () => {
  test("anchors are hidden until the panel's toggle is on", () => {
    const anchor = node({ kind: "anchor", tag: "" });
    expect(rowVisible(anchor, false)).toBe(false);
    expect(rowVisible(anchor, true)).toBe(true);
  });

  test("a whitespace-only text node never gets a row", () => {
    expect(rowVisible(node({ kind: "text", tag: "", text: "   \n" }), true)).toBe(false);
    expect(rowVisible(node({ kind: "text", tag: "", text: "" }), true)).toBe(false);
    expect(rowVisible(node({ kind: "text", tag: "", text: "hello" }), false)).toBe(true);
  });

  test("elements are always visible", () => {
    expect(rowVisible(node({ tag: "div" }), false)).toBe(true);
    expect(rowVisible(node({ tag: "root" }), false)).toBe(true);
  });
});
