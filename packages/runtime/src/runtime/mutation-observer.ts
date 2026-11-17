import type { MirrorCollection, MirrorElement, MirrorNode } from "./types";
import { NO_NODES, NodeListProto, branded, staticList, touchDom } from "./collections";
import { attrChanged, noAttrNodes } from "./attributes";
import { defineGlobal } from "./globals";
import { warnCompat } from "../compat";
import { appRoot } from "./root";

interface MutationRecordLike {
  type: "childList" | "attributes";
  target: MirrorNode;
  addedNodes: MirrorCollection<MirrorNode>;
  removedNodes: MirrorCollection<MirrorNode>;
  attributeName: string | null;
  attributeNamespace: null;
  oldValue: string | null;
  previousSibling: null;
  nextSibling: null;
}
interface MutationTargetEntry {
  node: MirrorElement;
  subtree: boolean;
  options: {
    childList: boolean;
    attributes: boolean;
    attributeOldValue: boolean;
    attributeFilter: string[] | null;
  };
}
interface MutationObserverInitLike {
  subtree?: boolean;
  childList?: boolean;
  attributes?: boolean;
  attributeOldValue?: boolean;
  attributeFilter?: string[];
  characterData?: boolean;
}
interface MutationObserverLike {
  _callback: (records: MutationRecordLike[], ob: MutationObserverLike) => void;
  _targets: MutationTargetEntry[];
  _records: MutationRecordLike[];
  takeRecords(): MutationRecordLike[];
}

export const observers = new Set<MutationObserverLike>();
let mutationFlushQueued = false;

export function notifyMutation(
  target: MirrorNode,
  type: "childList" | "attributes",
  extra?: {
    addedNodes?: MirrorNode[];
    removedNodes?: MirrorNode[];
    attributeName?: string;
    oldValue?: string | null;
  },
): void {
  touchDom();
  if (type === "attributes" && !noAttrNodes)
    attrChanged(target as MirrorElement, extra!.attributeName!, extra!.oldValue ?? null);
  if (observers.size === 0) return;
  for (const ob of observers) {
    for (const entry of ob._targets) {
      const inScope = entry.node === target || (entry.subtree && isAncestor(entry.node, target));
      if (!inScope) continue;
      if (type === "childList" && !entry.options.childList) continue;
      if (type === "attributes") {
        if (!entry.options.attributes) continue;
        if (entry.options.attributeFilter && !entry.options.attributeFilter.includes(extra!.attributeName!)) {
          continue;
        }
      }
      ob._records.push(
        branded("MutationRecord", {
          type,
          target,
          addedNodes: staticList(NodeListProto, extra?.addedNodes ?? NO_NODES),
          removedNodes: staticList(NodeListProto, extra?.removedNodes ?? NO_NODES),
          attributeName: extra?.attributeName ?? null,
          attributeNamespace: null,
          oldValue: entry.options.attributeOldValue ? (extra?.oldValue ?? null) : null,
          previousSibling: null,
          nextSibling: null,
        }),
      );
      break; // one record per observer per mutation, as in the DOM
    }
  }
  if (!mutationFlushQueued) {
    mutationFlushQueued = true;
    Promise.resolve().then(flushMutations);
  }
}

function isAncestor(ancestor: MirrorNode, node: MirrorNode | null | undefined): boolean {
  for (let n: MirrorElement | null | undefined = node?.parentNode; n; n = n.parentNode) if (n === ancestor) return true;
  return false;
}

function flushMutations() {
  mutationFlushQueued = false;
  for (const ob of [...observers]) {
    if (!ob._records.length) continue;
    const records = ob.takeRecords();
    try {
      ob._callback(records, ob);
    } catch (e) {
      console.error(`[skathi] MutationObserver callback threw: ${e}`);
    }
  }
}

defineGlobal(
  "MutationObserver",
  class MutationObserver implements MutationObserverLike {
    // `declare` so the fields are ERASED rather than emitted as a constructor
    // prologue that would run before the assignments below.
    declare _callback: MutationObserverLike["_callback"];
    declare _targets: MutationTargetEntry[];
    declare _records: MutationRecordLike[];
    constructor(callback: MutationObserverLike["_callback"]) {
      this._callback = callback;
      this._targets = [];
      this._records = [];
    }
    observe(node: unknown, options: MutationObserverInitLike = {}) {
      // `observe(document, {subtree: true})` is how libraries (reka, floating-ui)
      // say "watch everything". A Document is not an element here, so watch the
      // root. Duck-typed: the same code runs in the test harness's private world.
      const root =
        node && (node as MirrorNode).kind !== 1 ? (node as { documentElement?: unknown }).documentElement : null;
      if (root) node = (root as MirrorNode).kind === 1 ? root : (appRoot ?? node);
      if (!node || (node as MirrorNode).kind !== 1) {
        warnCompat(
          "MutationObserver:target",
          "MutationObserver.observe() was given something that is not a rendered element — " +
            "only elements (and `document`, which is mapped to the root element) have a " +
            "subtree in Skathi, so nothing under it is observed",
        );
        return;
      }
      if (options.characterData) {
        warnCompat(
          "MutationObserver:characterData",
          "MutationObserver characterData records are not delivered — the retained tree carries " +
            "text as node content, not as a mutation; childList and attributes are",
        );
      }
      this._targets.push({
        node: node as MirrorElement,
        subtree: !!options.subtree,
        options: {
          childList: !!options.childList,
          attributes: !!options.attributes || !!options.attributeFilter,
          attributeOldValue: !!options.attributeOldValue,
          attributeFilter: options.attributeFilter ?? null,
        },
      });
      observers.add(this);
      __sk_style_observe(true);
    }
    disconnect() {
      this._targets.length = 0;
      this._records.length = 0;
      observers.delete(this);
      if (!observers.size) __sk_style_observe(false);
    }
    takeRecords() {
      return this._records.splice(0, this._records.length);
    }
  },
);
