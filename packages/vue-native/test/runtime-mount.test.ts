import { describe, expect, test } from "bun:test";
import { h, Teleport } from "@vue/runtime-core";
import { createRuntime } from "./runtime-harness";
import { htmlPreludeModule } from "../vitecompat";

function htmlContainer(rt: ReturnType<typeof createRuntime>) {
  const source = htmlPreludeModule({
    headCss: [],
    bodyCss: [],
    scripts: [],
    rootClasses: ["dark"],
    rootAttrs: {},
    mountElement: { tag: "div", attrs: { id: "app", class: "application", "vaul-drawer-wrapper": "" } },
    advisories: [],
  });
  new Function("document", source.replace(/^import .*;$/gm, ""))(rt.global.document);
  const body = rt.global.document.body;
  const container = body.children[0];
  rt.host.queryAnswers.set("#app", String(container.__vnId));
  rt.host.queryAnswers.set('[id="app"]', String(container.__vnId));
  return { body, container };
}

const elements = (parent: any): any[] => Array.from(parent.children);

describe("native and browser mount containers", () => {
  test("index #app is a body child and its wrapper attributes do not transform body portals", () => {
    const rt = createRuntime();
    const { body, container } = htmlContainer(rt);
    expect(body.classList.contains("dark")).toBe(true);
    expect(body.hasAttribute("vaul-drawer-wrapper")).toBe(false);
    expect(container.className).toBe("application");
    expect(container.hasAttribute("vaul-drawer-wrapper")).toBe(true);
    const app = rt.createApp({
      setup: () => ({ marker: "mounted" }),
      render: () =>
        h("main", null, [h("p", null, "background"), h(Teleport, { to: "body" }, [h("aside", null, "drawer")])]),
    });
    const vm = app.mount("#app");
    expect(vm.marker).toBe("mounted");
    expect(container.parentNode).toBe(body);
    expect(elements(container).map((el) => el.tag)).toEqual(["main"]);
    const portal = elements(body).find((el) => el.tag === "aside");
    expect(portal).toBeDefined();
    expect(container.contains(portal)).toBe(false);
    expect(portal.parentNode).toBe(body);
    app.unmount();
    expect(container.parentNode).toBe(body);
    expect(container.childNodes).toEqual([]);
    expect(elements(body)).toEqual([container]);
    rt.global.__VN_TEARDOWN__();
    expect(rt.nodesById.size).toBe(0);
    expect(rt.looseById.size).toBe(0);
    expect(rt.looseRoots.size).toBe(0);
  });

  test("an Element argument is honored, including body insertions made before Vue mounts", () => {
    const rt = createRuntime();
    rt.global.document.documentElement.style.setProperty("--radius", "8px");
    const container = rt.global.document.createElement("section");
    container.id = "custom-container";
    rt.global.document.body.appendChild(container);
    const body = rt.global.document.body;
    expect(body.style.getPropertyValue("--radius")).toBe("8px");
    rt.createApp({ render: () => h("button", null, "inside") }).mount(container);
    expect(container.children[0].tag).toBe("button");
    expect(elements(body)).toEqual([container]);
    rt.global.__VN_TEARDOWN__();
    expect(container.__vnId).toBe(0);
    expect(body.__vnId).toBe(0);
  });

  test("an explicit mount replaces placeholder children and applies Vue's container markers", () => {
    const rt = createRuntime();
    const { container } = htmlContainer(rt);
    const placeholder = rt.global.document.createElement("p");
    placeholder.textContent = "Loading";
    container.appendChild(placeholder);
    container.setAttribute("v-cloak", "");
    rt.createApp({ render: () => h("main", null, "Ready") }).mount("#app");
    expect(container.textContent).toBe("Ready");
    expect(placeholder.parentNode).toBeNull();
    expect(placeholder.isConnected).toBe(false);
    expect(container.hasAttribute("v-cloak")).toBe(false);
    expect(container.getAttribute("data-v-app")).toBe("");
    rt.global.__VN_UNLOAD__();
    expect(rt.nodesById.size).toBe(0);
    expect(rt.looseRoots.size).toBe(0);
  });

  test("mount() and a legacy root-only #app Teleport keep working without index markup", () => {
    const rt = createRuntime();
    const app = rt.createApp({
      render: () => h("main", null, [h(Teleport, { to: "#app" }, [h("aside", null, "legacy")])]),
    });
    expect(app.mount()).toBeDefined();
    const body = rt.global.document.body;
    expect(body).toBe(rt.global.document.documentElement);
    expect(body.hasAttribute("data-v-app")).toBe(false);
    expect(
      elements(body)
        .map((el) => el.tag)
        .sort(),
    ).toEqual(["aside", "main"]);
    rt.global.__VN_TEARDOWN__();
    expect(rt.nodesById.size).toBe(0);
  });

  test("mount('#app') without index markup uses the native root and survives same-realm reload", () => {
    let rt = createRuntime();
    let previousBody: any;
    for (const generation of [1, 2]) {
      const preserved = rt.global.document.createElement("nav");
      preserved.textContent = "native sibling";
      rt.global.document.body.appendChild(preserved);
      const app = rt.createApp({
        setup: () => ({ generation }),
        render: () =>
          h("main", null, [
            h("span", null, `app ${generation}`),
            h(Teleport, { to: "body" }, [h("aside", null, "portal")]),
          ]),
      });
      expect(app.mount("#app").generation).toBe(generation);
      const body = rt.global.document.body;
      expect(body).toBe(rt.global.document.documentElement);
      expect(
        elements(body)
          .map((el) => el.tag)
          .sort(),
      ).toEqual(["aside", "main", "nav"]);
      expect(preserved.parentNode).toBe(body);
      expect(body.hasAttribute("data-v-app")).toBe(false);
      expect(elements(body).some((el) => el.attrs.id === "app")).toBe(false);
      expect(rt.advisories().some((message) => message.includes("app.mount"))).toBe(false);
      if (previousBody) expect(body).not.toBe(previousBody);
      rt.global.__VN_UNLOAD__();
      expect(body.__vnId).toBe(0);
      expect(rt.nodesById.size).toBe(0);
      previousBody = body;
      if (generation === 1) rt = createRuntime({ realm: rt.global });
    }
  });

  test("same-realm reload rebuilds exactly one app container with body portals outside it", () => {
    const first = createRuntime();
    const old = htmlContainer(first);
    first
      .createApp({ render: () => h("main", null, [h(Teleport, { to: "body" }, [h("aside", null, "old drawer")])]) })
      .mount(old.container);
    first.global.__VN_UNLOAD__();
    expect(first.nodesById.size).toBe(0);
    expect(old.body.__vnId).toBe(0);
    expect(old.container.__vnId).toBe(0);
    const second = createRuntime({ realm: first.global });
    const current = htmlContainer(second);
    second
      .createApp({ render: () => h("main", null, [h(Teleport, { to: "body" }, [h("aside", null, "new drawer")])]) })
      .mount(current.container);
    expect(elements(current.body).filter((el) => el.attrs.id === "app")).toEqual([current.container]);
    const drawer = elements(current.body).find((el) => el.tag === "aside");
    expect(drawer.textContent).toBe("new drawer");
    expect(current.container.contains(drawer)).toBe(false);
    second.global.__VN_TEARDOWN__();
  });

  test("unload also frees an index-created root if the entry fails before Vue mounts", () => {
    const first = createRuntime();
    const old = htmlContainer(first);
    expect(old.container.parentNode).toBe(old.body);
    first.global.__VN_UNLOAD__();
    expect(first.nodesById.size).toBe(0);
    expect(first.looseById.size).toBe(0);
    expect(old.container.__vnId).toBe(0);
    expect(old.body.__vnId).toBe(0);
    const second = createRuntime({ realm: first.global });
    const current = htmlContainer(second);
    second.createApp({ render: () => h("main", null, "recovered") }).mount(current.container);
    expect(elements(current.body)).toEqual([current.container]);
    expect(current.container.textContent).toBe("recovered");
    second.global.__VN_UNLOAD__();
    expect(second.nodesById.size).toBe(0);
  });

  test("a missing selector does not silently render into the viewport instead", () => {
    const rt = createRuntime();
    const app = rt.createApp({ render: () => h("main", null, "unexpected") });
    expect(app.mount("#missing")).toBeUndefined();
    expect(rt.global.document.body.childNodes).toEqual([]);
    expect(rt.advisories().some((message) => message.includes("found no element"))).toBe(true);
  });
});
