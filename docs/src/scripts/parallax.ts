let parallaxBody: HTMLElement | undefined;

function placeSides(sides: HTMLElement[]): void {
  const sidebar = document.querySelector(".sidebar")?.getBoundingClientRect();
  const main = document.querySelector(".main-content")?.getBoundingClientRect();
  const toc = document.querySelector(".right-sidebar")?.getBoundingClientRect();
  const width = document.documentElement.clientWidth;
  const sidebarRight = sidebar && sidebar.right > 0 ? sidebar.right : 0;
  // Clear of the reading rail, which hangs about 40px left of the article.
  const left = { x: sidebarRight + 12, end: main ? main.left - 52 : 0 };
  const right = { x: (toc && toc.width > 0 ? toc.right : (main?.right ?? width)) + 12, end: width - 8 };
  const bounds = sides.map((side) => (side.dataset["side"] === "left" ? left : right));
  sides.forEach((side, index) => {
    const box = bounds[index];
    const room = box ? box.end - box.x : 0;
    side.classList.toggle("is-open", room >= 40);
    if (box && room >= 40) {
      side.style.setProperty("--side-x", `${Math.round(box.x)}px`);
      side.style.setProperty("--side-width", `${Math.round(room)}px`);
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
  const sides = [...layer.querySelectorAll<HTMLElement>("[data-side]")];
  const docs = layer.dataset["parallax"] === "docs";
  const reach = docs ? 40 : 60;
  const layers = [...layer.querySelectorAll<HTMLElement>(".parallax-layer")].map((element) => ({
    element,
    depth: Number(element.style.getPropertyValue("--depth")) || 0,
  }));
  const moving = !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let measure = sides.length > 0;
  let frame = 0;
  let px = 0;
  let py = 0;
  const update = () => {
    frame = 0;
    if (measure) {
      measure = false;
      placeSides(sides);
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
  if (sides.length > 0)
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
