import type { TransitionBeforeSwapEvent } from "astro:transitions/client";

let initializedBody: HTMLElement | undefined;
let pageEvents: AbortController | undefined;

function initializePage(): void {
  if (initializedBody === document.body) return;
  initializedBody = document.body;
  pageEvents?.abort();
  pageEvents = new AbortController();
  const { signal } = pageEvents;
  const root = document.documentElement;
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  let preference = "system";
  try {
    preference = localStorage.getItem("skathi-theme") ?? "system";
  } catch {}

  function setTheme(value: string): void {
    preference = value;
    const theme = value === "system" ? (media.matches ? "dark" : "light") : value;
    root.dataset["theme"] = theme;
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute("content", theme === "dark" ? "#17161e" : "#faf9f6");
    document.querySelectorAll<HTMLButtonElement>("[data-theme-choice]").forEach((button) => {
      button.setAttribute("aria-pressed", String(button.dataset["themeChoice"] === value));
    });
  }

  setTheme(preference);
  document.querySelectorAll<HTMLButtonElement>("[data-theme-choice]").forEach((button) => {
    button.addEventListener(
      "click",
      () => {
        const choice = button.dataset["themeChoice"] ?? "system";
        setTheme(choice);
        try {
          localStorage.setItem("skathi-theme", choice);
        } catch {}
      },
      { signal },
    );
  });
  media.addEventListener(
    "change",
    () => {
      if (preference === "system") setTheme("system");
    },
    { signal },
  );

  const menu = document.getElementById("menu-toggle");
  const backdrop = document.getElementById("nav-backdrop");
  const shell = document.querySelector<HTMLElement>(".docs-shell");
  function closeNav(): void {
    const wasOpen = root.classList.contains("nav-open");
    root.classList.remove("nav-open");
    menu?.setAttribute("aria-expanded", "false");
    menu?.setAttribute("aria-label", "Open navigation");
    if (backdrop) backdrop.hidden = true;
    if (shell) shell.inert = false;
    if (wasOpen) menu?.focus();
  }
  menu?.addEventListener(
    "click",
    () => {
      if (root.classList.contains("nav-open")) {
        closeNav();
        return;
      }
      root.classList.add("nav-open");
      menu.setAttribute("aria-expanded", "true");
      menu.setAttribute("aria-label", "Close navigation");
      if (backdrop) backdrop.hidden = false;
      if (shell) shell.inert = true;
    },
    { signal },
  );
  backdrop?.addEventListener("click", closeNav, { signal });
  window.addEventListener(
    "keydown",
    (event) => {
      if (event.key === "Escape") closeNav();
      if (event.key !== "Tab" || !root.classList.contains("nav-open")) return;
      const controls = [
        ...document.querySelectorAll<HTMLElement>(
          ".mobile-header button, .sidebar a, .sidebar button, .sidebar summary",
        ),
      ].filter((element) => element.getClientRects().length > 0);
      const first = controls[0];
      const last = controls.at(-1);
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      }
      if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    },
    { signal },
  );
  window.matchMedia("(min-width: 701px)").addEventListener(
    "change",
    (event) => {
      if (event.matches) closeNav();
    },
    { signal },
  );
  document
    .getElementById("mobile-search")
    ?.addEventListener("click", () => window.dispatchEvent(new Event("skathi:search")), { signal });

  const tocLinks = [...document.querySelectorAll<HTMLAnchorElement>("[data-toc-id]")];
  const allTocLinks = document.querySelectorAll<HTMLAnchorElement>(".toc-link");
  const headingElements = tocLinks
    .map((link) => document.getElementById(link.dataset["tocId"] ?? ""))
    .filter((heading): heading is HTMLElement => heading !== null);
  let ticking = false;
  function updateToc(): void {
    const active = headingElements.filter((heading) => heading.getBoundingClientRect().top <= 130).at(-1);
    allTocLinks.forEach((link) => {
      const selected = active ? link.dataset["tocId"] === active.id : link.hasAttribute("data-toc-overview");
      link.classList.toggle("is-active", selected);
      if (selected) link.setAttribute("aria-current", "location");
      else link.removeAttribute("aria-current");
    });
    ticking = false;
  }
  window.addEventListener(
    "scroll",
    () => {
      if (!ticking) {
        ticking = true;
        window.requestAnimationFrame(updateToc);
      }
    },
    { passive: true, signal },
  );
  updateToc();

  document.querySelectorAll<HTMLPreElement>(".prose pre").forEach((pre) => {
    const code = pre.querySelector("code");
    if (!code) return;
    const wrapper = document.createElement("div");
    wrapper.className = "code-block";
    pre.before(wrapper);
    const toolbar = document.createElement("div");
    toolbar.className = "code-toolbar";
    const language = document.createElement("span");
    language.textContent = pre.dataset["language"] ?? code.dataset["language"] ?? "Code";
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = "Copy";
    button.setAttribute("aria-label", "Copy code");
    const status = document.createElement("span");
    status.className = "sr-only";
    status.setAttribute("role", "status");
    button.addEventListener(
      "click",
      async () => {
        try {
          await navigator.clipboard.writeText(code.textContent ?? "");
          button.textContent = "Copied!";
          status.textContent = "Code copied.";
        } catch {
          button.textContent = "Select code";
          const range = document.createRange();
          range.selectNodeContents(code);
          const selection = window.getSelection();
          selection?.removeAllRanges();
          selection?.addRange(range);
          status.textContent = "Code selected. Press Control C or Command C to copy.";
        }
        window.setTimeout(() => {
          button.textContent = "Copy";
          status.textContent = "";
        }, 2500);
      },
      { signal },
    );
    toolbar.append(language, button, status);
    wrapper.append(toolbar, pre);
  });

  document.querySelectorAll<HTMLButtonElement>("[data-copy]").forEach((button) => {
    const label = button.querySelector<HTMLElement>("[data-copy-label]");
    button.addEventListener(
      "click",
      async () => {
        try {
          await navigator.clipboard.writeText(button.dataset["copy"] ?? "");
          if (label) label.textContent = "Copied!";
        } catch {
          if (label) label.textContent = "Press Ctrl C";
        }
        window.setTimeout(() => {
          if (label) label.textContent = "Copy";
        }, 2000);
      },
      { signal },
    );
  });

  document.querySelectorAll<HTMLTableElement>(".prose table").forEach((table) => {
    const wrapper = document.createElement("div");
    wrapper.className = "table-scroll";
    wrapper.tabIndex = 0;
    wrapper.setAttribute("role", "region");
    wrapper.setAttribute("aria-label", "Scrollable reference table");
    table.before(wrapper);
    wrapper.append(table);
  });
}

document.addEventListener("astro:before-swap", (event: TransitionBeforeSwapEvent) => {
  event.newDocument.documentElement.dataset["theme"] = document.documentElement.dataset["theme"] ?? "dark";
});
document.addEventListener("astro:page-load", initializePage);
initializePage();
