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

function startParallax(signal: AbortSignal): void {
  const layer = document.querySelector<HTMLElement>("[data-parallax]");
  if (!layer || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  let frame = 0;
  let px = 0;
  let py = 0;
  const update = () => {
    frame = 0;
    layer.style.setProperty("--scroll", String(window.scrollY));
    layer.style.setProperty("--px", px.toFixed(3));
    layer.style.setProperty("--py", py.toFixed(3));
  };
  const schedule = () => {
    if (!frame) frame = window.requestAnimationFrame(update);
  };
  window.addEventListener("scroll", schedule, { passive: true, signal });
  window.addEventListener(
    "pointermove",
    (event) => {
      if (event.pointerType !== "mouse") return;
      px = event.clientX / window.innerWidth - 0.5;
      py = event.clientY / window.innerHeight - 0.5;
      schedule();
    },
    { passive: true, signal },
  );
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
  startParallax(signal);

  const tabs = [...document.querySelectorAll<HTMLButtonElement>("[data-shot]")];
  for (const tab of tabs) {
    tab.addEventListener(
      "click",
      () => {
        for (const other of tabs) {
          const selected = other === tab;
          other.setAttribute("aria-selected", String(selected));
          const panel = document.getElementById(other.getAttribute("aria-controls") ?? "");
          if (panel) panel.hidden = !selected;
        }
      },
      { signal },
    );
  }

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
