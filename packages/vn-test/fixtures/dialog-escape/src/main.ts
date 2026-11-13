import { createApp, h, nextTick, onMounted, onUnmounted, ref } from "vue-native";

const EXIT_MS = 250;
const OPEN_DELAY_MS = 100;

type State = { log: string[]; layers: string[]; active(): string };
const out: State = {
  log: [],
  layers: [],
  active: () => (document.activeElement as { id?: string } | null)?.id || "body",
};
(globalThis as unknown as { __VN_TEST_ESCAPE__: State }).__VN_TEST_ESCAPE__ = out;

const tipOpen = ref(false);
const menuOpen = ref(false);
const menuMounted = ref(false);
const dialogOpen = ref(false);

function useLayer(name: string, close: () => void) {
  const onKey = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || out.layers[out.layers.length - 1] !== name) return;
    out.log.push(`escape:${name}`);
    close();
  };
  onMounted(() => {
    out.layers.push(name);
    window.addEventListener("keydown", onKey);
  });
  onUnmounted(() => {
    out.layers.splice(out.layers.indexOf(name), 1);
    window.removeEventListener("keydown", onKey);
  });
}

function useFocusScope(name: string, container: () => HTMLElement | null, first: () => HTMLElement | null, trapped: boolean) {
  let previous: HTMLElement | null = null;
  let last: HTMLElement | null = null;
  const onIn = (e: FocusEvent) => {
    const c = container();
    if (!c) return;
    const t = e.target as HTMLElement;
    if (c.contains(t)) last = t;
    else last?.focus();
  };
  const onOut = (e: FocusEvent) => {
    const c = container();
    if (!c || e.relatedTarget === null) return;
    if (!c.contains(e.relatedTarget as HTMLElement)) last?.focus();
  };
  onMounted(() => {
    previous = document.activeElement as HTMLElement | null;
    out.log.push(`scope:${name} remembers ${out.active()}`);
    if (trapped) {
      document.addEventListener("focusin", onIn);
      document.addEventListener("focusout", onOut);
    }
    nextTick(() => {
      last = first();
      last?.focus();
    });
  });
  onUnmounted(() => {
    if (trapped) {
      document.removeEventListener("focusin", onIn);
      document.removeEventListener("focusout", onOut);
    }
    const back = previous;
    setTimeout(() => {
      out.log.push(`scope:${name} returns to ${(back as { id?: string } | null)?.id || "body"}`);
      back?.focus();
    }, 0);
  });
}

const note = (e: FocusEvent) => out.log.push(`${e.type}:${(e.currentTarget as { id?: string }).id}`);

let pressing = false;
const Tag = {
  render: () =>
    h(
      "span",
      {
        id: "tag",
        tabindex: 0,
        onFocus: (e: FocusEvent) => {
          note(e);
          if (!pressing) tipOpen.value = true;
        },
        onBlur: (e: FocusEvent) => {
          note(e);
          tipOpen.value = false;
        },
        onPointerdown: () => {
          tipOpen.value = false;
          pressing = true;
          document.addEventListener("pointerup", () => setTimeout(() => (pressing = false), 1), { once: true });
        },
        style: { padding: "4px 8px", background: "#ccd", borderRadius: "4px" },
      },
      "BOT",
    ),
};

const Tip = {
  setup() {
    useLayer("tip", () => (tipOpen.value = false));
    return () => h("div", { id: "tip", style: { position: "fixed", left: "24px", top: "8px", padding: "4px", background: "#333", color: "#fff" } }, "a bot");
  },
};

const Menu = {
  setup() {
    const el = ref<HTMLElement | null>(null);
    useLayer("menu", () => closeMenu());
    // floating-ui measures the anchor on mount, before the FocusScope reads
    // `document.activeElement`.
    onMounted(() => document.getElementById("row")?.getBoundingClientRect());
    useFocusScope("menu", () => el.value, () => el.value, false);
    return () =>
      h(
        "div",
        {
          id: "menu",
          ref: el,
          tabindex: -1,
          role: "menu",
          style: { position: "fixed", left: "40px", top: "90px", padding: "4px", background: "#eee", opacity: menuOpen.value ? 1 : 0.5 },
        },
        [
          h(
            "div",
            {
              id: "report",
              role: "menuitem",
              tabindex: -1,
              onClick: () => {
                closeMenu();
                setTimeout(() => (dialogOpen.value = true), OPEN_DELAY_MS);
              },
              style: { padding: "6px 12px", background: "#ddd" },
            },
            "Report",
          ),
        ],
      );
  },
};

function closeMenu() {
  if (!menuOpen.value) return;
  menuOpen.value = false;
  setTimeout(() => (menuMounted.value = false), EXIT_MS);
}

const Dialog = {
  setup() {
    const el = ref<HTMLElement | null>(null);
    const btn = ref<HTMLElement | null>(null);
    useLayer("dialog", () => (dialogOpen.value = false));
    useFocusScope("dialog", () => el.value, () => btn.value, true);
    return () =>
      h(
        "div",
        {
          id: "dialog",
          ref: el,
          role: "dialog",
          style: { position: "fixed", left: "160px", top: "60px", width: "200px", padding: "16px", background: "#fff", border: "2px solid #888" },
        },
        [h("button", { id: "dlg-btn", ref: btn }, "Spam")],
      );
  },
};

const App = {
  render: () =>
    h("div", { id: "page", style: { position: "relative", height: "100%", padding: "16px", background: "#f8f8f8" } }, [
      h(
        "div",
        {
          id: "row",
          onContextmenu: (e: MouseEvent) => {
            e.preventDefault();
            menuOpen.value = true;
            menuMounted.value = true;
          },
          style: { display: "flex", gap: "8px", alignItems: "center", padding: "8px", background: "#e4e4f0", width: "120px" },
        },
        ["member", h(Tag)],
      ),
      h("button", { id: "outside", onFocus: note, onBlur: note, style: { marginTop: "200px" } }, "outside"),
      tipOpen.value ? h(Tip) : null,
      menuMounted.value ? h(Menu) : null,
      dialogOpen.value ? h(Dialog) : null,
    ]),
};

createApp(App).mount("#app");
