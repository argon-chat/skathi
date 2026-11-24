const compact = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });

async function refreshStats(signal: AbortSignal): Promise<void> {
  try {
    const response = await fetch("/api/stats", { signal });
    const body: unknown = response.ok ? await response.json() : undefined;
    if (typeof body !== "object" || body === null) return;
    for (const [key, value] of Object.entries(body)) {
      if (typeof value !== "number") continue;
      for (const label of document.querySelectorAll<HTMLElement>(`[data-stat-text="${key}"]`)) {
        label.textContent = compact.format(value);
        label.hidden = false;
        const cell = label.closest<HTMLElement>("[data-stat-cell]");
        if (cell) cell.hidden = false;
      }
    }
  } catch {}
}

function startGallery(signal: AbortSignal): void {
  const gallery = document.querySelector<HTMLElement>("[data-fan]");
  if (!gallery) return;
  const cards = [...gallery.querySelectorAll<HTMLElement>("[data-fan-card]")];
  const tabs = [...gallery.querySelectorAll<HTMLButtonElement>("[data-fan-tab]")];
  const videos = [...gallery.querySelectorAll<HTMLVideoElement>("video")];
  const stacked = window.matchMedia("(max-width: 900px)");
  const visible = new Set<HTMLVideoElement>();
  let active = Math.max(
    0,
    cards.findIndex((card) => card.classList.contains("is-active")),
  );
  const still =
    window.matchMedia("(prefers-reduced-motion: reduce)").matches ||
    (navigator as Navigator & { connection?: { saveData?: boolean } }).connection?.saveData === true;
  const attach = (video: HTMLVideoElement): void => {
    const pending = video.querySelectorAll<HTMLSourceElement>("source[data-src]");
    if (pending.length === 0) return;
    for (const source of pending) {
      source.src = source.dataset["src"] ?? "";
      source.removeAttribute("data-src");
    }
    video.load();
  };

  for (const video of videos) {
    if (still) {
      attach(video);
      video.controls = true;
    }
  }
  // Side by side only the middle window plays; stacked in a column, whichever are on screen.
  const play = (): void => {
    if (still) return;
    cards.forEach((card, index) => {
      const video = card.querySelector("video");
      if (!video) return;
      if (visible.has(video) && (stacked.matches || index === active)) {
        attach(video);
        void video.play().catch(() => {});
      } else {
        video.pause();
      }
    });
  };
  if (!still) {
    const watcher = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const video = entry.target as HTMLVideoElement;
          if (entry.isIntersecting) visible.add(video);
          else visible.delete(video);
        }
        play();
      },
      { rootMargin: "200px 0px" },
    );
    for (const video of videos) watcher.observe(video);
    signal.addEventListener("abort", () => watcher.disconnect());
    stacked.addEventListener("change", play, { signal });
    gallery.dataset["auto"] = "";
  }

  const activate = (index: number): void => {
    active = index;
    gallery.style.setProperty("--active", String(index));
    cards.forEach((card, at) => card.classList.toggle("is-active", at === index));
    tabs.forEach((tab, at) => tab.setAttribute("aria-pressed", String(at === index)));
    play();
  };
  const choose = (index: number): void => {
    delete gallery.dataset["auto"];
    activate(index);
  };
  tabs.forEach((tab, index) => tab.addEventListener("click", () => choose(index), { signal }));
  cards.forEach((card, index) =>
    card.addEventListener(
      "click",
      () => {
        if (index !== active && !stacked.matches) choose(index);
      },
      { signal },
    ),
  );
  gallery.addEventListener(
    "animationend",
    (event) => {
      if (event.animationName === "fan-progress") activate((active + 1) % cards.length);
    },
    { signal },
  );

  let frame = 0;
  const update = () => {
    frame = 0;
    const travel = gallery.offsetHeight - window.innerHeight;
    const raw = travel > 0 ? -gallery.getBoundingClientRect().top / (travel * 0.75) : 1;
    const t = Math.min(1, Math.max(0, raw));
    gallery.style.setProperty("--fan", (t * t * (3 - 2 * t)).toFixed(4));
    gallery.classList.toggle("is-open", t >= 1);
  };
  const schedule = () => {
    if (!frame) frame = window.requestAnimationFrame(update);
  };
  window.addEventListener("scroll", schedule, { passive: true, signal });
  window.addEventListener("resize", schedule, { passive: true, signal });
  signal.addEventListener("abort", () => window.cancelAnimationFrame(frame));
  update();
}

function startTabs(group: HTMLElement, signal: AbortSignal): void {
  const tabs = [...group.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
  const select = (tab: HTMLButtonElement): void => {
    for (const other of tabs) {
      const on = other === tab;
      other.setAttribute("aria-selected", String(on));
      other.tabIndex = on ? 0 : -1;
      const panel = document.getElementById(other.getAttribute("aria-controls") ?? "");
      if (panel) panel.hidden = !on;
    }
  };
  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => select(tab), { signal });
    tab.addEventListener(
      "keydown",
      (event) => {
        const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
        const next = tabs[(index + step + tabs.length) % tabs.length];
        if (!step || !next) return;
        event.preventDefault();
        select(next);
        next.focus();
      },
      { signal },
    );
  });
}

function initializeLanding(): void {
  const page = document.querySelector<HTMLElement>(".landing");
  if (!page || page.dataset["ready"]) return;
  page.dataset["ready"] = "true";
  const events = new AbortController();
  document.addEventListener("astro:before-swap", () => events.abort(), { once: true });
  const { signal } = events;

  void refreshStats(signal);
  startGallery(signal);
  for (const group of document.querySelectorAll<HTMLElement>("[data-tabs]")) startTabs(group, signal);

  const counter = document.querySelector<HTMLButtonElement>("[data-counter]");
  const files = [...document.querySelectorAll<HTMLElement>("[data-demo-code] [role='tabpanel']")];
  const line = (file: HTMLElement, number: number): HTMLElement | undefined =>
    file.querySelectorAll<HTMLElement>(".line")[number - 1];
  let count = 0;
  const showCount = (): void => {
    for (const file of files) {
      const state = line(file, Number(file.dataset["stateLine"]));
      if (state) state.dataset["value"] = String(count);
    }
  };
  showCount();
  counter?.addEventListener(
    "click",
    () => {
      count += 1;
      counter.textContent = `Clicked ${count} ${count === 1 ? "time" : "times"}`;
      showCount();
      for (const file of files) {
        for (const number of (file.dataset["hitLines"] ?? "").split(" ")) {
          const hit = line(file, Number(number));
          if (!hit) continue;
          hit.classList.remove("is-hit");
          void hit.offsetWidth;
          hit.classList.add("is-hit");
        }
      }
    },
    { signal },
  );
}

document.addEventListener("astro:page-load", initializeLanding);
initializeLanding();
