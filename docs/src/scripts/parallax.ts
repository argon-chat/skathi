let parallaxBody: HTMLElement | undefined;

/** The columns of the viewport that no page element covers at any scroll position. */
function freeColumns(): { x: number; end: number }[] {
  const margin = 16;
  const blocked: [number, number][] = [[document.documentElement.clientWidth - 8, Infinity]];
  const block = (element: Element, before = margin) => {
    const r = element.getBoundingClientRect();
    if (r.width > 0 && r.height > 0) blocked.push([r.left - before, r.right + margin]);
  };
  document
    .querySelectorAll(".sidebar, .right-sidebar, .prose .wide, .prose [data-rail-bend]")
    .forEach((el) => block(el));
  // Clear of the reading rail, which hangs about 40px left of the article.
  const main = document.querySelector(".main-content");
  if (main) block(main, 52);
  blocked.sort((a, b) => a[0] - b[0]);
  const free: { x: number; end: number }[] = [];
  let x = 0;
  for (const [start, end] of blocked) {
    if (start - x >= 40) free.push({ x, end: start });
    x = Math.max(x, end);
  }
  return free;
}

function placeZones(zones: HTMLElement[]): void {
  const free = freeColumns();
  zones.forEach((zone, index) => {
    const column = free[index];
    zone.classList.toggle("is-open", column !== undefined);
    if (column) {
      zone.style.setProperty("--zone-x", `${Math.round(column.x)}px`);
      zone.style.setProperty("--zone-width", `${Math.round(column.end - column.x)}px`);
    }
  });
}

function startParallax(): void {
  if (parallaxBody === document.body) return;
  parallaxBody = document.body;
  const layer = document.querySelector<HTMLElement>("[data-parallax]");
  if (!layer) return;
  const events = new AbortController();
  document.addEventListener("astro:before-swap", () => events.abort(), { once: true });
  const { signal } = events;
  const zones = [...layer.querySelectorAll<HTMLElement>("[data-zone]")];
  const docs = layer.dataset["parallax"] === "docs";
  const reach = docs ? 40 : 60;
  const layers = [...layer.querySelectorAll<HTMLElement>(".parallax-layer")].map((element) => ({
    element,
    depth: Number(element.style.getPropertyValue("--depth")) || 0,
  }));
  const moving = !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let measure = zones.length > 0;
  let frame = 0;
  let px = 0;
  let py = 0;
  const update = () => {
    frame = 0;
    if (measure) {
      measure = false;
      placeZones(zones);
    }
    if (!moving) return;
    for (const { element, depth } of layers) {
      const drift = window.scrollY * depth;
      // The docs layers repeat every viewport height, so they wrap instead of running out on long pages.
      const y = (docs ? -(drift % window.innerHeight) : -drift) + py * depth * reach;
      element.style.transform = `translate3d(${px * depth * reach}px, ${y}px, 0)`;
    }
  };
  const schedule = () => {
    if (!frame) frame = window.requestAnimationFrame(update);
  };
  if (moving) {
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
  }
  if (zones.length > 0)
    window.addEventListener(
      "resize",
      () => {
        measure = true;
        schedule();
      },
      { passive: true, signal },
    );
  signal.addEventListener("abort", () => window.cancelAnimationFrame(frame));
  update();
}

document.addEventListener("astro:page-load", startParallax);
startParallax();
