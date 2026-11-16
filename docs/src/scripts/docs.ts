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
  const allTocLinks = [...document.querySelectorAll<HTMLAnchorElement>(".toc-link")];
  const toc = document.querySelector<HTMLElement>(".toc");
  // Looked up on every update: the catalog island re-renders its group headings when filters change.
  const headingElements = () =>
    tocLinks
      .map((link) => document.getElementById(link.dataset["tocId"] ?? ""))
      .filter((heading): heading is HTMLElement => heading !== null && heading.getClientRects().length > 0);

  document.querySelectorAll<HTMLHeadingElement>(".prose h2[id], .prose h3[id]").forEach((heading) => {
    if (heading.querySelector(".heading-anchor")) return;
    const anchor = document.createElement("a");
    anchor.className = "heading-anchor";
    anchor.href = `#${heading.id}`;
    anchor.setAttribute("aria-label", `Link to ${heading.textContent ?? "this section"}`);
    anchor.innerHTML =
      '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M10 14a4.5 4.5 0 0 0 6.4 0l3.2-3.2a4.5 4.5 0 0 0-6.4-6.4L12 5.6M14 10a4.5 4.5 0 0 0-6.4 0l-3.2 3.2a4.5 4.5 0 0 0 6.4 6.4l1.2-1.2"/></svg>';
    heading.append(anchor);
  });

  const prose = document.querySelector<HTMLElement>(".prose");
  const rail = prose ? document.createElement("div") : undefined;
  const svgNS = "http://www.w3.org/2000/svg";
  const railSvg = document.createElementNS(svgNS, "svg");
  const railTrack = document.createElementNS(svgNS, "path");
  const railFill = document.createElementNS(svgNS, "path");
  const railGradient = document.createElementNS(svgNS, "linearGradient");
  let railNodes: { node: HTMLElement; heading: HTMLElement; top: number }[] = [];
  let railTop = 0;
  let railHeight = 0;
  let railLength = 0;
  const lineX = 4.5;
  function railPath(): string {
    if (!rail) return "";
    const box = rail.getBoundingClientRect();
    const bend = 32;
    const clearance = 16;
    let y = 0;
    let d = `M${lineX} 0`;
    const frames = [...document.querySelectorAll<HTMLElement>(".prose .demo-frame")]
      .map((frame) => frame.getBoundingClientRect())
      .filter((r) => r.height > 0 && r.left < box.left + lineX + clearance)
      .map((r) => ({ top: r.top - box.top, bottom: r.bottom - box.top, x: r.left - box.left - clearance }))
      .sort((a, b) => a.top - b.top);
    for (const frame of frames) {
      const start = frame.top - clearance - bend;
      const end = frame.bottom + clearance + bend;
      if (start < y || end > railHeight) continue;
      const mid = bend / 2;
      d += ` V${start} C${lineX} ${start + mid} ${frame.x} ${start + mid} ${frame.x} ${start + bend}`;
      d += ` V${end - bend} C${frame.x} ${end - mid} ${lineX} ${end - mid} ${lineX} ${end}`;
      y = end;
    }
    return `${d} V${railHeight}`;
  }
  function lengthAtDepth(depth: number): number {
    let low = 0;
    let high = railLength;
    for (let i = 0; i < 18; i++) {
      const mid = (low + high) / 2;
      if (railTrack.getPointAtLength(mid).y < depth) low = mid;
      else high = mid;
    }
    return high;
  }
  function layoutRail(): void {
    if (!prose || !rail) return;
    const sections = [...prose.querySelectorAll<HTMLElement>(":scope h2")].filter(
      (heading) => heading.getClientRects().length > 0,
    );
    rail.hidden = sections.length < 2;
    if (rail.hidden) return;
    const origin = prose.getBoundingClientRect().top;
    const first = sections[0];
    railTop = first ? first.getBoundingClientRect().top - origin + 18 : 0;
    railHeight = Math.max(0, prose.getBoundingClientRect().height - railTop - 8);
    rail.style.setProperty("--rail-top", `${railTop}px`);
    rail.style.setProperty("--rail-height", `${railHeight}px`);
    const d = railPath();
    railTrack.setAttribute("d", d);
    railFill.setAttribute("d", d);
    railLength = railTrack.getTotalLength();
    rail.style.setProperty("--rail-length", `${railLength}`);
    railFill.style.strokeDasharray = `${railLength} ${railLength}`;
    railNodes.forEach(({ node }) => node.remove());
    railNodes = sections.map((heading) => {
      const node = document.createElement("span");
      node.className = "rail-node";
      const top = heading.getBoundingClientRect().top - origin + 18;
      node.style.top = `${top - railTop}px`;
      rail.append(node);
      return { node, heading, top };
    });
    updateReading();
  }
  if (prose && rail) {
    rail.className = "rail";
    rail.setAttribute("aria-hidden", "true");
    railGradient.id = "rail-gradient";
    railGradient.setAttribute("gradientUnits", "userSpaceOnUse");
    railGradient.setAttribute("x1", "0");
    railGradient.setAttribute("x2", "0");
    for (const name of ["from", "to"]) {
      const stop = document.createElementNS(svgNS, "stop");
      stop.setAttribute("class", `rail-stop-${name}`);
      stop.setAttribute("offset", name === "from" ? "0" : "1");
      railGradient.append(stop);
    }
    const defs = document.createElementNS(svgNS, "defs");
    defs.append(railGradient);
    railTrack.setAttribute("class", "rail-track");
    railFill.setAttribute("class", "rail-fill");
    railSvg.append(defs, railTrack, railFill);
    rail.append(railSvg);
    prose.prepend(rail);
    prose.classList.add("has-rail");
    new ResizeObserver(() => layoutRail()).observe(prose);
  }

  let ticking = false;
  function updateReading(): void {
    const line = window.innerHeight * 0.32;
    if (prose && rail && !rail.hidden) {
      const reading = line - prose.getBoundingClientRect().top;
      const filled = Math.min(Math.max(reading - railTop, 0), railHeight);
      const length = filled <= 0 ? 0 : filled >= railHeight ? railLength : lengthAtDepth(filled);
      railFill.style.strokeDashoffset = `${railLength - length}`;
      railGradient.setAttribute("y1", "0");
      railGradient.setAttribute("y2", `${Math.max(filled, 1)}`);
      railNodes.forEach(({ node, heading, top }) => {
        const passed = reading >= top;
        node.classList.toggle("is-passed", passed);
        heading.classList.toggle("is-reached", passed);
      });
    }
    const active = headingElements()
      .filter((heading) => heading.getBoundingClientRect().top <= line)
      .at(-1);
    let selectedLink: HTMLAnchorElement | undefined;
    allTocLinks.forEach((link) => {
      const selected = active ? link.dataset["tocId"] === active.id : link.hasAttribute("data-toc-overview");
      link.classList.toggle("is-active", selected);
      if (selected) {
        link.setAttribute("aria-current", "location");
        selectedLink = link;
      } else link.removeAttribute("aria-current");
    });
    if (toc && selectedLink) {
      toc.style.setProperty("--toc-y", `${selectedLink.offsetTop}px`);
      toc.style.setProperty("--toc-h", `${selectedLink.offsetHeight}px`);
      toc.classList.add("has-marker");
    }
    ticking = false;
  }
  window.addEventListener(
    "scroll",
    () => {
      if (!ticking) {
        ticking = true;
        window.requestAnimationFrame(updateReading);
      }
    },
    { passive: true, signal },
  );
  window.addEventListener("resize", layoutRail, { passive: true, signal });
  document.fonts?.ready.then(layoutRail);
  layoutRail();
  updateReading();

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
    button.className = "code-copy";
    button.setAttribute("aria-label", "Copy code");
    button.innerHTML =
      '<svg class="code-copy-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path class="icon-copy" d="M9 9h11v11H9V9Zm-4 6H4V4h11v1"/><path class="icon-check" d="m5 12 5 5L20 7"/></svg><span class="code-copy-label">Copy</span>';
    const label = button.querySelector<HTMLElement>(".code-copy-label");
    const status = document.createElement("span");
    status.className = "sr-only";
    status.setAttribute("role", "status");
    let reset: number | undefined;
    button.addEventListener(
      "click",
      async () => {
        window.clearTimeout(reset);
        try {
          await navigator.clipboard.writeText(code.textContent ?? "");
          button.dataset["state"] = "copied";
          if (label) label.textContent = "Copied";
          status.textContent = "Code copied.";
        } catch {
          if (label) label.textContent = "Select code";
          const range = document.createRange();
          range.selectNodeContents(code);
          const selection = window.getSelection();
          selection?.removeAllRanges();
          selection?.addRange(range);
          status.textContent = "Code selected. Press Control C or Command C to copy.";
        }
        reset = window.setTimeout(() => {
          delete button.dataset["state"];
          if (label) label.textContent = "Copy";
          status.textContent = "";
        }, 2200);
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

  document.querySelectorAll<HTMLQuoteElement>(".prose blockquote:not([data-kind])").forEach((quote) => {
    const lead = quote.querySelector(":scope > p:first-child > strong:first-child")?.textContent?.replace(/:$/, "");
    const kind = lead?.toLowerCase();
    quote.classList.add("callout");
    if (kind === "warning" || kind === "caution" || kind === "important") quote.dataset["kind"] = "warning";
    else if (kind === "tip") quote.dataset["kind"] = "tip";
    else quote.dataset["kind"] = "note";
  });

  document.querySelectorAll<HTMLTableElement>(".prose table:not([data-plain])").forEach((table) => {
    const wrapper = document.createElement("div");
    wrapper.className = "table-scroll";
    wrapper.tabIndex = 0;
    wrapper.setAttribute("role", "region");
    wrapper.setAttribute("aria-label", "Scrollable reference table");
    table.before(wrapper);
    wrapper.append(table);
  });

  const demos = [...document.querySelectorAll<HTMLVideoElement>("video[data-demo]")];
  if (demos.length) {
    const attach = (video: HTMLVideoElement): void => {
      const pending = video.querySelectorAll<HTMLSourceElement>("source[data-src]");
      if (!pending.length) return;
      for (const source of pending) {
        source.src = source.dataset["src"] ?? "";
        source.removeAttribute("data-src");
      }
      video.load();
    };
    const still =
      window.matchMedia("(prefers-reduced-motion: reduce)").matches ||
      (navigator as Navigator & { connection?: { saveData?: boolean } }).connection?.saveData === true;
    if (still)
      for (const video of demos) {
        attach(video);
        video.controls = true;
      }
    else {
      const watcher = new IntersectionObserver(
        (entries) => {
          for (const entry of entries) {
            const video = entry.target as HTMLVideoElement;
            if (entry.isIntersecting) {
              attach(video);
              void video.play().catch(() => {});
            } else video.pause();
          }
        },
        { rootMargin: "200px 0px" },
      );
      for (const video of demos) watcher.observe(video);
      signal.addEventListener("abort", () => watcher.disconnect());
    }
  }
}

document.addEventListener("astro:before-swap", (event: TransitionBeforeSwapEvent) => {
  event.newDocument.documentElement.dataset["theme"] = document.documentElement.dataset["theme"] ?? "dark";
});
document.addEventListener("astro:page-load", initializePage);
initializePage();
