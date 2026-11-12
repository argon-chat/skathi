// A fixture for packages/vn-test/specs/iframe.spec.ts: the `<iframe>` element
// as a page that embeds a sandboxed game uses it (research/reports/m26/
// iframe-webview-requirements-2026-11-12.md §2).
//
// The page: `#slot`, an empty 400 × 300 box, and `#plain`, an `<iframe>` in the
// template with no attributes. `__VN_TEST_IFRAME__.mount(src)` does what
// Argon's PlayFrame host does, in its order: create the element, set
// `sandbox.value`, `allow`, `referrerPolicy` and the style, append it, attach
// `onload` / `onerror` and a `message` listener on `window`, and only then set
// `src`. Everything that happens is appended to `log`, which `state()` returns
// as JSON with what the spec reads off the element.
import { createApp, h } from "vue-native";

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any
const g = globalThis as Any;

const App = {
  render: () =>
    h("main", { id: "page" }, [
      h("div", { id: "slot", style: "position: relative; width: 400px; height: 300px" }),
      h("iframe", { id: "plain" }),
    ]),
};
createApp(App).mount("#app");

const log: string[] = [];
let frame: Any = null;
let firstWindow: Any = null;

window.addEventListener("message", (e: Any) => {
  const mine = frame && e.source === frame.contentWindow;
  // A bigint has no JSON form; the log is text.
  const text = JSON.stringify(e.data, (_k, v) => (typeof v === "bigint" ? `${v}n` : v));
  log.push(`message ${mine ? "mine" : "other"} ${e.origin} ${text}`);
});

g.__VN_TEST_IFRAME__ = {
  mount(src: string): void {
    const slot = document.getElementById("slot") as Any;
    frame = document.createElement("iframe");
    log.push(`detached contentWindow ${frame.contentWindow === null ? "null" : "set"}`);
    frame.sandbox.value = "allow-scripts allow-pointer-lock";
    frame.allow = "fullscreen 'none'";
    frame.referrerPolicy = "origin";
    frame.setAttribute("loading", "eager");
    frame.style.border = "none";
    frame.style.display = "block";
    frame.style.position = "absolute";
    frame.style.left = "20px";
    frame.style.top = "10px";
    frame.style.width = "320px";
    frame.style.height = "240px";
    slot.appendChild(frame);
    firstWindow = frame.contentWindow;
    frame.onload = () => log.push("load");
    frame.onerror = () => log.push("error");
    frame.src = src;
  },

  /** Posts as the host does; returns what the call threw, if anything. */
  post(): string {
    const out: string[] = [];
    const tryPost = (label: string, fn: () => void) => {
      try {
        fn();
        out.push(`${label} ok`);
      } catch (e) {
        out.push(`${label} ${(e as Error).name}`);
      }
    };
    tryPost("star", () => frame.contentWindow.postMessage({ type: "ping", n: 1 }, "*"));
    tryPost("options", () => frame.contentWindow.postMessage({ type: "ping", n: 2 }, { targetOrigin: "*" }));
    tryPost("function", () => frame.contentWindow.postMessage({ fn() {} }, "*"));
    tryPost("node", () => frame.contentWindow.postMessage({ el: frame }, "*"));
    tryPost("transfer", () => frame.contentWindow.postMessage(1, "*", [new ArrayBuffer(1)]));
    return out.join("; ");
  },

  /** Moves the element to the end of `#page` and back, as a `<Teleport>` toggle does. */
  move(): void {
    const page = document.getElementById("page") as Any;
    const slot = document.getElementById("slot") as Any;
    page.appendChild(frame);
    slot.appendChild(frame);
  },

  unmount(): void {
    frame?.remove();
    log.push(`removed contentWindow ${frame.contentWindow === null ? "null" : "set"}`);
  },

  state(): string {
    const plain = document.getElementById("plain") as Any;
    const rect = (el: Any) => {
      const r = el.getBoundingClientRect();
      return [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)];
    };
    const slot = (document.getElementById("slot") as Any).getBoundingClientRect();
    const f = frame;
    return JSON.stringify({
      supported: g.vnFeatures?.iframe,
      log,
      plain: {
        isIframe: plain instanceof HTMLIFrameElement,
        tagName: plain.tagName,
        size: rect(plain).slice(2),
        src: plain.src,
        sandbox: plain.sandbox.value,
        contentDocument: plain.contentDocument,
      },
      frame: f && {
        sandbox: [f.sandbox.value, f.sandbox.length, f.sandbox.contains("allow-scripts"), f.sandbox.contains("allow-same-origin"), f.getAttribute("sandbox")],
        sandboxAssign: (() => {
          const probe = document.createElement("iframe") as Any;
          probe.sandbox = "allow-forms";
          probe.sandbox.add("allow-scripts", "allow-forms");
          probe.sandbox.remove("allow-forms");
          return [probe.getAttribute("sandbox"), probe.sandbox.supports("allow-scripts"), probe.sandbox.supports("nope")];
        })(),
        attrs: [f.allow, f.getAttribute("allow"), f.referrerPolicy, f.getAttribute("referrerpolicy"), f.getAttribute("loading"), f.src],
        // Relative to the slot: the host positions the frame inside its container.
        box: f.isConnected ? [Math.round(rect(f)[0] - slot.left), Math.round(rect(f)[1] - slot.top), rect(f)[2], rect(f)[3]] : null,
        sameWindow: f.isConnected ? f.contentWindow === firstWindow && f.contentWindow === f.contentWindow : null,
        windowKeys: f.isConnected ? [typeof f.contentWindow.postMessage, String(f.contentWindow.document), Object.getPrototypeOf(f.contentWindow)] : null,
      },
    });
  },
};
