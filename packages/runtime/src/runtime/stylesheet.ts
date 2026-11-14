import { mark, original, vacant } from "./realm";
import { warnCompat } from "../compat";
import { HTMLCollectionProto, NodeListProto, liveCollection, staticList, touchDom } from "./collections";
import { stylesheetText } from "./tree-ops";
import { hooks } from "./state";
import { elementsUnder, tagTest } from "./dom-members";
import { appRoot } from "../runtime";

// `document.head` is not a box, so a head `<style>` is a stylesheet, not a tree node: its text joins the app's sheet
// after the bundle's CSS, in insertion order, and follows later edits. Before mount it rides the first
// `__sk_set_stylesheet`; after, each change re-sets the sheet once per microtask. A `<link rel=stylesheet>` in head is
// not fetched (advisory).
export let installStylesheet: (css: string) => void = (css) => __sk_set_stylesheet(css);
{
  const d = globalThis.document as unknown as Record<string, unknown> | undefined;
  const head = d?.head as Record<string, unknown> | undefined;
  // The HOST's function, even after a reload (the global of that name is then the previous runtime's `install`; see `original`).
  const hostSetSheet =
    typeof __sk_set_stylesheet === "function" ? original(globalThis, "__sk_set_stylesheet", __sk_set_stylesheet) : null;
  if (d && head && typeof head === "object" && hostSetSheet) {
    const injected: { el: Record<string, unknown>; text: string }[] = [];
    let base: string | null = null;
    let queued = false;
    // Each injected sheet goes to the host as its own text: the host keeps one parse per distinct text, so concatenating
    // would re-parse the whole bundle sheet on every change.
    const install = (css: string) => {
      base = String(css);
      hostSetSheet(base, injected.map((s) => s.text).filter(Boolean));
    };
    // Every installer goes through this (the mount, loader.ts's flushCss via the global), so injected sheets survive a hot reload's re-aggregation.
    installStylesheet = install;
    try {
      Object.defineProperty(globalThis, "__sk_set_stylesheet", { value: install, configurable: true, writable: true });
    } catch {
      warnCompat(
        "head-style:install",
        "the dev loader's stylesheet updates will drop <style> elements added to document.head",
      );
    }
    const refresh = () => {
      touchDom(); // `getElementsByTagName("style")` lists them
      if (base === null || queued) return; // before the first sheet: the mount's install carries them
      queued = true;
      queueMicrotask(() => {
        queued = false;
        if (base !== null) install(base);
      });
    };
    const textOf = stylesheetText;
    const isStyle = (n: unknown) =>
      !!n && typeof n === "object" && String((n as { tagName?: unknown }).tagName ?? "").toUpperCase() === "STYLE";
    const isLink = (n: unknown) =>
      !!n && typeof n === "object" && String((n as { tagName?: unknown }).tagName ?? "").toUpperCase() === "LINK";
    const track = (el: Record<string, unknown>) => {
      const entry = { el, text: textOf(el) };
      injected.push(entry);
      const set = (v: unknown) => {
        entry.text = v == null ? "" : String(v);
        refresh();
      };
      try {
        for (const prop of ["textContent", "innerHTML", "innerText", "text"]) {
          Object.defineProperty(el, prop, { get: () => entry.text, set, configurable: true });
        }
        const append = (child: unknown) => {
          entry.text += textOf(child);
          refresh();
          return child;
        };
        Object.defineProperty(el, "appendChild", { value: append, configurable: true, writable: true });
        Object.defineProperty(el, "append", {
          value: (...kids: unknown[]) => kids.forEach(append),
          configurable: true,
          writable: true,
        });
        Object.defineProperty(el, "remove", { value: () => untrack(el), configurable: true, writable: true });
        Object.defineProperty(el, "parentNode", {
          get: () => (injected.includes(entry) ? head : null),
          configurable: true,
        });
      } catch {
        warnCompat("head-style:track", "a <style> in document.head will not follow later edits to its text");
      }
      refresh();
    };
    const untrack = (el: unknown) => {
      const i = injected.findIndex((s) => s.el === el);
      if (i >= 0) {
        injected.splice(i, 1);
        refresh();
      }
    };
    const adopt = (n: unknown) => {
      if (isStyle(n)) {
        if (!injected.some((s) => s.el === n)) track(n as Record<string, unknown>);
      } else if (isLink(n)) {
        const rel = String(
          (n as { rel?: unknown }).rel ?? (n as { getAttribute?(k: string): unknown }).getAttribute?.("rel") ?? "",
        );
        if (/stylesheet/i.test(rel)) {
          warnCompat(
            "head-link",
            "a <link rel=stylesheet> added to document.head is not loaded — Skathi has no network stylesheets; import the CSS instead",
          );
        }
      }
      return n;
    };
    const def = (name: string, value: unknown) =>
      Object.defineProperty(head, name, { value, configurable: true, writable: true });
    def("appendChild", adopt);
    def("insertBefore", (n: unknown) => adopt(n));
    def("append", (...kids: unknown[]) => void kids.forEach(adopt));
    def("prepend", (...kids: unknown[]) => void kids.forEach(adopt));
    def("insertAdjacentElement", (_where: unknown, n: unknown) => adopt(n));
    def("removeChild", (n: unknown) => {
      untrack(n);
      return n;
    });
    if (!("tagName" in head)) def("tagName", "HEAD");
    // A previous runtime's getter (same-realm reload, see `REALM`) read ITS injected list, so it is replaced.
    if (vacant(head, "childNodes")) {
      Object.defineProperty(head, "childNodes", {
        get: mark(() =>
          staticList(
            NodeListProto,
            injected.map((s) => s.el),
          ),
        ),
        configurable: true,
      });
    }

    hooks.foreignStyle = (n, inserted) => {
      if (!isStyle(n) && !isLink(n)) return false;
      if (inserted) adopt(n);
      else untrack(n);
      return true;
    };
    if (vacant(d, "getElementsByTagName")) {
      Object.defineProperty(d, "getElementsByTagName", {
        value: mark((tag: unknown) => {
          const t = String(tag).toLowerCase();
          const test = tagTest(tag);
          return liveCollection<unknown>(HTMLCollectionProto, true, () => {
            if (t === "head") return [head];
            if (t === "html" || t === "body") return [d.documentElement];
            const tree = elementsUnder(appRoot, false, test);
            return t === "style" ? [...injected.map((s) => s.el), ...tree] : tree;
          });
        }),
        configurable: true,
        writable: true,
      });
    }
  }
}
