// The inline style as ONE CSSOM declaration block: `:style`, `el.style`,
// `style.cssText` and the `style` attribute are four views of it, and every
// change reaches the host through `__vn_set_style` (docs/RUNTIME.md §5.3).

import { describe, expect, test } from "bun:test";
import { createRuntime } from "./runtime-harness";

function setup() {
  const rt = createRuntime();
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
