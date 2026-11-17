const compact = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });

async function refreshStats(signal: AbortSignal): Promise<void> {
  try {
    const response = await fetch("/api/stats", { signal });
    const body: unknown = response.ok ? await response.json() : undefined;
    if (typeof body !== "object" || body === null) return;
    for (const [key, value] of Object.entries(body)) {
      const cell = document.querySelector<HTMLElement>(`[data-stat-cell="${key}"]`);
      const number = cell?.querySelector("dd");
      if (!cell || !number || typeof value !== "number") continue;
      number.textContent = compact.format(value);
      cell.hidden = false;
    }
  } catch {}
}

function startGallery(signal: AbortSignal): void {
  const gallery = document.querySelector<HTMLElement>("[data-fan]");
  if (!gallery) return;
  const videos = [...gallery.querySelectorAll<HTMLVideoElement>("video")];
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
  if (!still) {
    const watcher = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const video = entry.target as HTMLVideoElement;
          if (entry.isIntersecting) {
            attach(video);
            void video.play().catch(() => {});
          } else {
            video.pause();
          }
        }
      },
      { rootMargin: "200px 0px" },
    );
    for (const video of videos) watcher.observe(video);
    signal.addEventListener("abort", () => watcher.disconnect());
  }

  let frame = 0;
  const update = () => {
    frame = 0;
    const travel = gallery.offsetHeight - window.innerHeight;
    const raw = travel > 0 ? -gallery.getBoundingClientRect().top / (travel * 0.75) : 1;
    const t = Math.min(1, Math.max(0, raw));
    gallery.style.setProperty("--fan", (t * t * (3 - 2 * t)).toFixed(4));
  };
  const schedule = () => {
    if (!frame) frame = window.requestAnimationFrame(update);
  };
  window.addEventListener("scroll", schedule, { passive: true, signal });
  window.addEventListener("resize", schedule, { passive: true, signal });
  signal.addEventListener("abort", () => window.cancelAnimationFrame(frame));
  update();
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

  const counter = document.querySelector<HTMLButtonElement>("[data-counter]");
  let count = 0;
  counter?.addEventListener(
    "click",
    () => {
      count += 1;
      counter.textContent = `Clicked ${count} ${count === 1 ? "time" : "times"}`;
    },
    { signal },
  );
}

document.addEventListener("astro:page-load", initializeLanding);
initializeLanding();
