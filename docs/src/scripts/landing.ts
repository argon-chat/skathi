const compact = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });

const statKeys = ["npm", "stars"] as const;

function isStats(value: unknown): value is Record<(typeof statKeys)[number], number> {
  if (typeof value !== "object" || value === null) return false;
  return statKeys.every((key) => typeof (value as Record<string, unknown>)[key] === "number");
}

async function refreshStats(signal: AbortSignal): Promise<void> {
  const strip = document.querySelector<HTMLElement>("[data-stats]");
  if (!strip) return;
  try {
    const response = await fetch("/api/stats", { signal });
    const body: unknown = response.ok ? await response.json() : undefined;
    if (!isStats(body)) return;
    for (const key of statKeys) {
      const cell = strip.querySelector<HTMLElement>(`[data-stat="${key}"]`);
      if (cell) cell.textContent = compact.format(body[key]);
    }
    strip.hidden = false;
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
