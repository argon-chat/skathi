// The inline style as ONE CSSOM declaration block: `:style`, `el.style`,
// `style.cssText` and the `style` attribute are four views of it, and every
// change reaches the host through `__vn_set_style` (docs/RUNTIME.md §5.3).

import { describe, expect, test } from "bun:test";
import { defineComponent, h, nextTick, ref } from "@vue/runtime-core";
import { createRuntime } from "./runtime-harness";

function setup(options: { styleAccepts?: boolean } = {}) {
  const rt = createRuntime(options);
  const root = rt.nodeOps.createElement("div");
  const el = rt.nodeOps.createElement("div");
  rt.nodeOps.insert(el, root, null);
  /** What the host holds for this element, in its order (Tree::set_style's rule). */
  const hostStyle = () => {
    const out: [string, string][] = [];
    for (const { id, key, value } of rt.host.styles) {
      if (id !== el.id) continue;
      const i = out.findIndex(([k]) => k === key);
      if (value === "") {
        if (i >= 0) out.splice(i, 1);
      } else if (i >= 0) out[i][1] = value;
      else out.push([key, value]);
    }
    return out;
  };
  return { rt, el, hostStyle };
}

describe("the style attribute is the inline declaration block", () => {
  test("getAttribute('style') serializes what Vue's :style set; null before anything was", () => {
    const { rt, el } = setup();
    expect(el.getAttribute("style")).toBe(null);
    expect(el.hasAttribute("style")).toBe(false);
    rt.patchProp(el, "style", null, { color: "red", width: "10px" });
    expect(el.getAttribute("style")).toBe("color: red; width: 10px;");
    expect(el.hasAttribute("style")).toBe(true);
    expect(el.getAttributeNames()).toContain("style");
    expect(el.style.cssText).toBe("color: red; width: 10px;");
  });

  test("setAttribute('style', s) replaces the block with the parse of s, and keeps s as the attribute", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, { color: "red", height: "4px" });
    el.setAttribute("style", "width:10px;color:blue");
    // The attribute is the text as written, as in a browser…
    expect(el.getAttribute("style")).toBe("width:10px;color:blue");
    // …the block is its parse, and the host holds exactly that.
    expect(el.style.cssText).toBe("width: 10px; color: blue;");
    expect(el.style.height).toBe("");
    expect(el.style.color).toBe("blue");
    expect(hostStyle()).toEqual([
      ["width", "10px"],
      ["color", "blue"],
    ]);
    // A CSSOM change re-serializes the attribute.
    el.style.color = "green";
    expect(el.getAttribute("style")).toBe("width: 10px; color: green;");
  });

  test("removeAttribute('style') clears the block on the host too", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, { color: "red" });
    el.removeAttribute("style");
    expect(el.getAttribute("style")).toBe(null);
    expect(el.hasAttribute("style")).toBe(false);
    expect(el.style.cssText).toBe("");
    expect(el.style.length).toBe(0);
    expect(el.getAttributeNames()).not.toContain("style");
    expect(hostStyle()).toEqual([]);
  });

  test("style.cssText gets and sets the same block", () => {
    const { el, hostStyle } = setup();
    el.style.cssText = "display: flex; gap: 4px";
    expect(el.getAttribute("style")).toBe("display: flex; gap: 4px;");
    expect(el.style.display).toBe("flex");
    expect(hostStyle()).toEqual([
      ["display", "flex"],
      ["gap", "4px"],
    ]);
    el.style.cssText = "gap: 8px";
    expect(hostStyle()).toEqual([["gap", "8px"]]);
    expect(el.style.display).toBe("");
  });

  test("hasAttribute follows the browser: present once written, even when emptied", () => {
    const { el } = setup();
    // Removing a property that is not there changes nothing, so no attribute.
    el.style.color = "";
    expect(el.hasAttribute("style")).toBe(false);
    el.style.color = "red";
    expect(el.hasAttribute("style")).toBe(true);
    el.style.removeProperty("color");
    // The attribute stays, empty.
    expect(el.hasAttribute("style")).toBe(true);
    expect(el.getAttribute("style")).toBe("");
    el.style.cssText = "";
    expect(el.getAttribute("style")).toBe("");
    el.removeAttribute("style");
    expect(el.hasAttribute("style")).toBe(false);
  });

  test("declaration order is CSSOM's: an update stays in place, a new property is appended", () => {
    const { el, hostStyle } = setup();
    el.style.color = "red";
    el.style.width = "1px";
    el.style.color = "blue";
    expect(el.style.cssText).toBe("color: blue; width: 1px;");
    expect(el.style.item(0)).toBe("color");
    expect(el.style[1]).toBe("width");
    expect(el.style[2]).toBe(undefined);
    expect([...el.style]).toEqual(["color", "width"]);
    expect(hostStyle()).toEqual([
      ["color", "blue"],
      ["width", "1px"],
    ]);
  });

  test("the parse: strings, parentheses and comments hide a `;`, and a nameless declaration is dropped", () => {
    const { el } = setup();
    el.setAttribute(
      "style",
      "background: url(\"data:image/png;base64,AAA=\") /* a;b */ no-repeat; 0: d; : x; color: ; --x: 3; width: 1px; width: 2px",
    );
    expect(el.style.background).toBe('url("data:image/png;base64,AAA=")   no-repeat');
    expect(el.style.getPropertyValue("--x")).toBe("3");
    expect(el.style.length).toBe(3);
    // The later duplicate wins, at its own position.
    expect([...el.style]).toEqual(["background", "--x", "width"]);
    expect(el.style.width).toBe("2px");
  });

  test("a normal duplicate does not override an earlier !important one", () => {
    const { el } = setup();
    el.style.cssText = "color: red !important; color: blue";
    expect(el.style.color).toBe("red");
    expect(el.style.getPropertyPriority("color")).toBe("important");
  });

  test("setProperty's priority, and what a browser ignores", () => {
    const { el, hostStyle } = setup();
    el.style.setProperty("color", "red", "important");
    expect(el.style.getPropertyPriority("color")).toBe("important");
    expect(el.getAttribute("style")).toBe("color: red !important;");
    expect(hostStyle()).toEqual([["color", "red !important"]]);
    // An unknown priority, and a priority inside the value, are ignored.
    el.style.setProperty("width", "1px", "bogus");
    el.style.setProperty("height", "1px !important");
    el.style.height = "2px !important";
    expect(el.style.width).toBe("");
    expect(el.style.height).toBe("");
    // `undefined` is no value; `null` is the empty string, which removes.
    el.style.color = undefined;
    expect(el.style.color).toBe("red");
    el.style.color = null;
    expect(el.style.color).toBe("");
    // setProperty takes CSS names: a camelCase name is not one.
    el.style.setProperty("backgroundColor", "red");
    expect(el.style.getPropertyValue("background-color")).toBe("");
    el.style.setProperty("Background-Color", "red");
    expect(el.style.getPropertyValue("background-color")).toBe("red");
  });

  test("camelCase, dashed and prefixed attribute names reach the same declaration", () => {
    const { el } = setup();
    el.style.backgroundColor = "red";
    expect(el.style["background-color"]).toBe("red");
    el.style.webkitTransform = "none";
    el.style.MozOsxFontSmoothing = "auto";
    el.style.cssFloat = "left";
    expect([...el.style]).toEqual(["background-color", "-webkit-transform", "-moz-osx-font-smoothing", "float"]);
    expect(el.style.cssFloat).toBe("left");
  });

  test("custom properties keep their case, and an empty one survives as one space", () => {
    const { el, hostStyle } = setup();
    el.style.setProperty("--rekaPopperWidth", "320px");
    el.style.setProperty("--empty", " ");
    expect(el.style.getPropertyValue("--rekaPopperWidth")).toBe("320px");
    expect(el.style.getPropertyValue("--rekapopperwidth")).toBe("");
    expect(hostStyle()).toEqual([
      ["--rekaPopperWidth", "320px"],
      ["--empty", " "],
    ]);
  });

  test("assigning a custom property works, and says that only setProperty does in a browser (S-2)", () => {
    const { rt, el, hostStyle } = setup();
    el.style["--gap"] = "4px";
    el.style["--gap"] = "6px";
    expect(el.style.getPropertyValue("--gap")).toBe("6px");
    expect(hostStyle()).toEqual([["--gap", "6px"]]);
    const said = rt.advisories().filter((a) => a.includes("expando"));
    expect(said.length).toBe(1);
    expect(said[0]).toContain('setProperty("--gap"');
  });

  test("a MutationObserver sees style changes, with the old value", async () => {
    const { rt, el } = setup();
    el.style.color = "red";
    const seen = [];
    const ob = new rt.global.MutationObserver((r) => seen.push(...r));
    ob.observe(el, { attributes: true, attributeOldValue: true, attributeFilter: ["style"] });
    el.style.color = "blue";
    el.setAttribute("style", "width:1px");
    el.removeAttribute("style");
    await Promise.resolve();
    await Promise.resolve();
    expect(seen.map((r) => [r.attributeName, r.oldValue])).toEqual([
      ["style", "color: red;"],
      ["style", "color: blue;"],
      ["style", "width:1px"],
    ]);
    ob.disconnect();
  });

  test("the serialization divergence is advised once, and never for the verbatim attribute", () => {
    const { rt, el } = setup();
    el.setAttribute("style", "color:red");
    el.getAttribute("style");
    expect(rt.advisories().filter((a) => a.includes("serialize"))).toEqual([]);
    el.style.cssText;
    el.getAttribute("style");
    el.style.width = "1px";
    el.getAttribute("style");
    const said = rt.advisories().filter((a) => a.includes("serialize"));
    expect(said.length).toBe(1);
    expect(said[0]).toContain("as they were written");
  });

  test("a block re-set in the same order costs a host call only for what changed", () => {
    const { rt, el } = setup();
    el.style.cssText = "color: red; width: 1px";
    const before = rt.host.styles.length;
    el.style.cssText = "color: red; width: 2px";
    expect(rt.host.styles.slice(before)).toEqual([{ id: el.id, key: "width", value: "2px" }]);
  });

  test("a block re-set in another order is re-sent, so the host's order is the block's", () => {
    const { el, hostStyle } = setup();
    el.style.cssText = "margin-top: 4px; margin: 0";
    el.style.cssText = "margin: 0; margin-top: 4px";
    expect(hostStyle()).toEqual([
      ["margin", "0"],
      ["margin-top", "4px"],
    ]);
  });

  test("documentElement.style written before mount is replayed under its CSS name", () => {
    const rt = createRuntime();
    const doc = rt.global.document;
    doc.documentElement.style.fontSize = "14px";
    doc.documentElement.style.setProperty("--radius", "1rem");
    expect(doc.documentElement.style.getPropertyValue("font-size")).toBe("14px");
    rt.createApp({ render: () => null }).mount();
    const root = doc.documentElement;
    expect(root.style.fontSize).toBe("14px");
    expect(root.style.getPropertyValue("--radius")).toBe("1rem");
    expect(rt.host.styles.filter((s) => s.id === root.id).map((s) => s.key)).toEqual(["font-size", "--radius"]);
  });
});

describe(":style is runtime-dom's patchStyle", () => {
  test("a string style is the declaration body, not a record read character by character", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, "display:flex;gap:4px");
    expect(hostStyle()).toEqual([
      ["display", "flex"],
      ["gap", "4px"],
    ]);
    expect(rt.host.styles.some((s) => /^\d+$/.test(s.key))).toBe(false);
    expect(el.style.cssText).toBe("display: flex; gap: 4px;");
  });

  test("h('div', { style: '…' }) renders the string style", () => {
    const rt = createRuntime();
    let el: any = null;
    rt.createApp({ render: () => h("div", { ref: (e: unknown) => (el = e), style: "display:flex;gap:4px" }) }).mount();
    expect(el.style.display).toBe("flex");
    expect(el.getAttribute("style")).toBe("display: flex; gap: 4px;");
    expect(rt.host.styles.filter((s) => s.id === el.id).map((s) => `${s.key}=${s.value}`)).toEqual([
      "display=flex",
      "gap=4px",
    ]);
  });

  test("string → string replaces the block", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, "color: red; width: 1px");
    rt.patchProp(el, "style", "color: red; width: 1px", "width: 2px");
    expect(hostStyle()).toEqual([["width", "2px"]]);
  });

  test("record → record diffs per property; a missing key or a null value removes it", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, { color: "red", width: "1px", height: "2px" });
    const before = rt.host.styles.length;
    rt.patchProp(el, "style", { color: "red", width: "1px", height: "2px" }, { color: "red", width: null, opacity: 0.5 });
    expect(hostStyle()).toEqual([
      ["color", "red"],
      ["opacity", "0.5"],
    ]);
    // The unchanged `color` cost no host call.
    expect(rt.host.styles.slice(before).some((s) => s.key === "color")).toBe(false);
  });

  test("record → string clears the record's properties", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, { color: "red", width: "1px" });
    rt.patchProp(el, "style", { color: "red", width: "1px" }, "height: 3px");
    expect(hostStyle()).toEqual([["height", "3px"]]);
    expect(el.getAttribute("style")).toBe("height: 3px;");
  });

  test("string → record clears what the string declared and the record lacks", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, "color: red; width: 1px; background-color: blue");
    rt.patchProp(el, "style", "color: red; width: 1px; background-color: blue", { width: "2px", backgroundColor: "green" });
    expect(hostStyle()).toEqual([
      ["width", "2px"],
      ["background-color", "green"],
    ]);
  });

  test("null or undefined after a style removes the attribute and every declaration", () => {
    for (const gone of [null, undefined]) {
      const { rt, el, hostStyle } = setup();
      rt.patchProp(el, "style", null, { color: "red" });
      rt.patchProp(el, "style", { color: "red" }, gone);
      expect(hostStyle()).toEqual([]);
      expect(el.hasAttribute("style")).toBe(false);
      const s = setup();
      s.rt.patchProp(s.el, "style", null, "color: red");
      s.rt.patchProp(s.el, "style", "color: red", gone);
      expect(s.hostStyle()).toEqual([]);
      expect(s.el.getAttribute("style")).toBe(null);
    }
  });

  test("custom properties keep their case, through a record and a string", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, { "--rekaPopperWidth": "320px", "--gap": 4 });
    expect(hostStyle()).toEqual([
      ["--rekaPopperWidth", "320px"],
      ["--gap", "4"],
    ]);
    rt.patchProp(el, "style", { "--rekaPopperWidth": "320px", "--gap": 4 }, "--Mixed: 1");
    expect(hostStyle()).toEqual([["--Mixed", "1"]]);
  });

  test("`!important` in a record value is the priority (runtime-dom's setProperty(…, 'important'))", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, { color: "red !important", width: "1px" });
    expect(el.style.getPropertyPriority("color")).toBe("important");
    expect(el.style.color).toBe("red");
    expect(el.getAttribute("style")).toBe("color: red !important; width: 1px;");
    expect(hostStyle()).toEqual([
      ["color", "red !important"],
      ["width", "1px"],
    ]);
    // Dropping the priority is a change.
    rt.patchProp(el, "style", { color: "red !important", width: "1px" }, { color: "red", width: "1px" });
    expect(el.style.getPropertyPriority("color")).toBe("");
    expect(hostStyle()[0]).toEqual(["color", "red"]);
  });

  test("an array is vendor fallbacks: the last entry the engine accepts wins", () => {
    const { rt, el, hostStyle } = setup();
    rt.host.styleRejects.add("display: -ms-flexbox");
    rt.patchProp(el, "style", null, { display: ["-webkit-box", "-ms-flexbox", "flex"] });
    expect(hostStyle()).toEqual([["display", "flex"]]);
    // The last one was accepted, so nothing earlier was even asked about.
    expect(rt.host.acceptCalls).toEqual(["display: flex"]);
    expect(rt.advisories().some((a) => a.includes("style ") || a.includes(":style"))).toBe(false);
  });

  test("an array whose last entries the engine rejects uses an earlier one, and says so", () => {
    const { rt, el, hostStyle } = setup();
    rt.host.styleRejects.add("display: -webkit-box");
    rt.host.styleRejects.add("display: grid-lanes");
    rt.patchProp(el, "style", null, { display: ["flex", "-webkit-box", "grid-lanes"] });
    expect(hostStyle()).toEqual([["display", "flex"]]);
    const said = rt.advisories().filter((a) => a.includes(":style display"));
    expect(said.length).toBe(1);
    expect(said[0]).toContain('uses "flex"');
  });

  test("an array: an empty entry removes, nothing accepted leaves the declaration as it was", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, { width: "1px" });
    rt.host.styleRejects.add("width: nope");
    rt.patchProp(el, "style", { width: "1px" }, { width: ["nope"] });
    expect(hostStyle()).toEqual([["width", "1px"]]);
    rt.patchProp(el, "style", { width: ["nope"] }, { width: ["2px", null] });
    expect(hostStyle()).toEqual([]);
    rt.patchProp(el, "style", { width: ["2px", null] }, { width: ["3px", ["4px !important"]] });
    expect(el.style.width).toBe("4px");
    expect(el.style.getPropertyPriority("width")).toBe("important");
  });

  test("on a host without __vn_style_accepts an array keeps its last entry, with an advisory", () => {
    const { rt, el, hostStyle } = setup({ styleAccepts: false });
    rt.patchProp(el, "style", null, { display: ["-webkit-box", "flex"] });
    expect(hostStyle()).toEqual([["display", "flex"]]);
    expect(rt.advisories().some((a) => a.includes("__vn_style_accepts is missing"))).toBe(true);
  });

  test("a string style keeps the variables useCssVars set", async () => {
    const rt = createRuntime();
    const style = ref<string>("color: red");
    let el: any = null;
    const Comp = defineComponent({
      setup() {
        rt.useCssVars(() => ({ "abc-gap": "4px" }));
        return () => h("div", { ref: (e: unknown) => (el = e), style: style.value });
      },
    });
    rt.createApp(Comp).mount();
    await nextTick();
    expect(el.style.getPropertyValue("--abc-gap")).toBe("4px");
    style.value = "color: blue";
    await nextTick();
    await nextTick();
    expect(el.style.color).toBe("blue");
    expect(el.style.getPropertyValue("--abc-gap")).toBe("4px");
  });
});

describe("v-show keeps its display handling", () => {
  test("hide and show restore the element's own display", () => {
    const { rt, el, hostStyle } = setup();
    rt.patchProp(el, "style", null, { display: "flex", gap: "4px" });
    rt.vShow.beforeMount(el, { value: false });
    expect(el.style.display).toBe("none");
    rt.vShow.updated(el, { value: true, oldValue: false });
    expect(el.style.display).toBe("flex");
    expect(hostStyle()).toEqual([
      ["display", "flex"],
      ["gap", "4px"],
    ]);
  });

  test("v-show under a <Transition> says the phases are not run (S-5)", () => {
    const { rt, el } = setup();
    rt.vShow.beforeMount(el, { value: true }, { transition: {} });
    expect(rt.advisories().some((a) => a.includes("v-show inside <Transition>"))).toBe(true);
  });

  test("an element without an inline display shows by removing it", () => {
    const { rt, el, hostStyle } = setup();
    rt.vShow.beforeMount(el, { value: false });
    expect(hostStyle()).toEqual([["display", "none"]]);
    rt.vShow.updated(el, { value: true, oldValue: false });
    expect(hostStyle()).toEqual([]);
    // An unchanged truthiness is no work at all.
    const n = rt.host.styles.length;
    rt.vShow.updated(el, { value: 1, oldValue: true });
    expect(rt.host.styles.length).toBe(n);
  });

  test("a :style change while hidden keeps it hidden, and the new display is what shows", () => {
    const { rt, el } = setup();
    rt.patchProp(el, "style", null, { display: "flex" });
    rt.vShow.beforeMount(el, { value: false });
    rt.patchProp(el, "style", { display: "flex" }, { display: "grid", color: "red" });
    expect(el.style.display).toBe("none");
    expect(el.style.color).toBe("red");
    rt.vShow.updated(el, { value: true, oldValue: false });
    expect(el.style.display).toBe("grid");
    // A string style while hidden, too.
    rt.vShow.updated(el, { value: false, oldValue: true });
    rt.patchProp(el, "style", { display: "grid", color: "red" }, "display: block; color: blue");
    expect(el.style.display).toBe("none");
    rt.vShow.updated(el, { value: true, oldValue: false });
    expect(el.style.display).toBe("block");
  });
});
