import { createApp, h } from "@skathi-native/runtime";

const dom = document;

interface WindowMessage extends Event {
  readonly source: unknown;
  readonly origin: string;
  readonly data: unknown;
}
const g = globalThis as unknown as { skFeatures?: { iframe?: unknown }; __SK_TEST_IFRAME__: unknown };

const App = {
  render: () =>
    h("main", { id: "page" }, [
      h("div", { id: "slot", style: "position: relative; width: 400px; height: 300px" }),
      h("iframe", { id: "plain" }),
    ]),
};
createApp(App).mount("#app");

const log: string[] = [];
let frame: HTMLIFrameElement | null = null;
let firstWindow: WindowProxy | null = null;

window.addEventListener("message", (ev) => {
  const e = ev as WindowMessage;
  const mine = frame && e.source === frame.contentWindow;
  const text = JSON.stringify(e.data, (_k, v) => (typeof v === "bigint" ? `${v}n` : v));
  log.push(`message ${mine ? "mine" : "other"} ${e.origin} ${text}`);
});

g.__SK_TEST_IFRAME__ = {
  mount(src: string): void {
    const slot = dom.getElementById("slot")!;
    frame = dom.createElement("iframe");
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
    tryPost("star", () => frame!.contentWindow!.postMessage({ type: "ping", n: 1 }, "*"));
    tryPost("options", () => frame!.contentWindow!.postMessage({ type: "ping", n: 2 }, { targetOrigin: "*" }));
    tryPost("function", () => frame!.contentWindow!.postMessage({ fn() {} }, "*"));
    tryPost("node", () => frame!.contentWindow!.postMessage({ el: frame }, "*"));
    tryPost("transfer", () => frame!.contentWindow!.postMessage(1, "*", [new ArrayBuffer(1)]));
    return out.join("; ");
  },

  move(): void {
    const page = dom.getElementById("page")!;
    const slot = dom.getElementById("slot")!;
    page.appendChild(frame!);
    slot.appendChild(frame!);
  },

  unmount(): void {
    frame?.remove();
    log.push(`removed contentWindow ${frame!.contentWindow === null ? "null" : "set"}`);
  },

  state(): string {
    const plain = dom.getElementById("plain") as HTMLIFrameElement;
    const rect = (el: HTMLElement): [number, number, number, number] => {
      const r = el.getBoundingClientRect();
      return [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)];
    };
    const slot = dom.getElementById("slot")!.getBoundingClientRect();
    const f = frame;
    return JSON.stringify({
      supported: g.skFeatures?.iframe,
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
        sandbox: [
          f.sandbox.value,
          f.sandbox.length,
          f.sandbox.contains("allow-scripts"),
          f.sandbox.contains("allow-same-origin"),
          f.getAttribute("sandbox"),
        ],
        sandboxAssign: (() => {
          const probe = dom.createElement("iframe");
          probe.sandbox = "allow-forms";
          probe.sandbox.add("allow-scripts", "allow-forms");
          probe.sandbox.remove("allow-forms");
          return [
            probe.getAttribute("sandbox"),
            probe.sandbox.supports("allow-scripts"),
            probe.sandbox.supports("nope"),
          ];
        })(),
        attrs: [
          f.allow,
          f.getAttribute("allow"),
          f.referrerPolicy,
          f.getAttribute("referrerpolicy"),
          f.getAttribute("loading"),
          f.src,
        ],
        box: f.isConnected
          ? [Math.round(rect(f)[0] - slot.left), Math.round(rect(f)[1] - slot.top), rect(f)[2], rect(f)[3]]
          : null,
        sameWindow: f.isConnected ? f.contentWindow === firstWindow && f.contentWindow === f.contentWindow : null,
        windowKeys: f.isConnected
          ? [
              typeof f.contentWindow!.postMessage,
              String(f.contentWindow!.document),
              Object.getPrototypeOf(f.contentWindow),
            ]
          : null,
      },
    });
  },
};
