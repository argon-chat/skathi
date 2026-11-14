import {
  Fragment,
  Static,
  getCurrentInstance,
  onBeforeUpdate,
  onMounted,
  onUnmounted,
  queuePostFlushCb,
  watch,
} from "@vue/runtime-core";
import type { VNode } from "@vue/runtime-core";
import type { MirrorElement, MirrorNode } from "./types";
import { nodeOps } from "./node-ops";
import { warnCompat } from "../compat";

/** Writes the `--hash-name` custom properties a `v-bind()` in `<style>` reads onto the component's root element(s); teleported children are found by `data-v-owner`. */
export function useCssVars(getter: (ctx: unknown) => Record<string, unknown>): void {
  const instance = getCurrentInstance() as unknown as CssVarsInstance | null;
  if (!instance) return;
  const doc = (globalThis as { document?: { querySelectorAll?(s: string): ArrayLike<MirrorElement> } }).document;
  const updateTeleports = (instance.ut = (vars: Record<string, unknown> = getter(instance.proxy)) => {
    const owned = doc?.querySelectorAll?.(`[data-v-owner="${instance.uid}"]`) ?? [];
    Array.from(owned).forEach((node) => setVarsOnNode(node, vars));
  });
  const setVars = () => {
    const vars = getter(instance.proxy);
    if (instance.ce) setVarsOnNode(instance.ce as MirrorElement, vars);
    else setVarsOnVNode(instance.subTree, vars);
    updateTeleports(vars);
  };
  onBeforeUpdate(() => {
    queuePostFlushCb(setVars);
  });
  onMounted(() => {
    watch(setVars, () => {}, { flush: "post" });
    const MO = (
      globalThis as {
        MutationObserver?: new (cb: () => void) => { observe(n: unknown, o: unknown): void; disconnect(): void };
      }
    ).MutationObserver;
    const parent = (instance.subTree?.el as MirrorNode | null)?.parentNode;
    if (MO && parent) {
      const ob = new MO(setVars);
      ob.observe(parent, { childList: true });
      onUnmounted(() => ob.disconnect());
    }
  });
}
interface CssVarsInstance {
  uid: number;
  proxy: unknown;
  ce?: unknown;
  subTree: VNode;
  ut?: (vars?: Record<string, unknown>) => void;
}
function setVarsOnVNode(vnode: VNode, vars: Record<string, unknown>): void {
  if (vnode.shapeFlag & 128 /* SUSPENSE */) {
    const suspense = vnode.suspense as unknown as {
      activeBranch: VNode;
      pendingBranch: VNode | null;
      isHydrating: boolean;
      effects: (() => void)[];
    };
    vnode = suspense.activeBranch;
    if (suspense.pendingBranch && !suspense.isHydrating) {
      suspense.effects.push(() => setVarsOnVNode(suspense.activeBranch, vars));
    }
  }
  while (vnode.component) vnode = vnode.component.subTree;
  if (vnode.shapeFlag & 1 /* ELEMENT */ && vnode.el) {
    setVarsOnNode(vnode.el as MirrorElement, vars);
  } else if (vnode.type === Fragment) {
    (vnode.children as VNode[]).forEach((c) => setVarsOnVNode(c, vars));
  } else if (vnode.type === Static) {
    let el = vnode.el as MirrorNode | null;
    const anchor = vnode.anchor as MirrorNode | null;
    while (el) {
      setVarsOnNode(el as MirrorElement, vars);
      if (el === anchor) break;
      el = nodeOps.nextSibling(el);
    }
  }
}
function setVarsOnNode(el: MirrorElement, vars: Record<string, unknown>): void {
  if (el.nodeType !== 1) return;
  // runtime-dom's `CSS_VAR_TEXT`: `patchStyle` appends this to a string
  // `:style`, so a component's `v-bind()` variables survive it.
  let cssText = "";
  for (const key in vars) {
    const value = normalizeCssVarValue(vars[key]);
    el.style.setProperty(`--${key}`, value);
    cssText += `--${key}: ${value};`;
  }
  el._cssVarText = cssText;
}
function normalizeCssVarValue(value: unknown): string {
  if (value == null) return "initial";
  if (typeof value === "string") return value === "" ? " " : value;
  return String(value);
}

export function useCssModule(name = "$style"): Record<string, string> {
  const instance = getCurrentInstance();
  if (!instance) return {};
  const modules = (instance.type as { __cssModules?: Record<string, Record<string, string>> }).__cssModules;
  const mod = modules?.[name];
  if (!mod) {
    warnCompat(
      `cssModule:${name}`,
      `useCssModule(${JSON.stringify(name)}) — no CSS module of that name: Skathi's builders do not ` +
        `compile <style module> yet, so this answers {} and every class lookup is undefined`,
    );
    return {};
  }
  return mod;
}
