let pupBody: HTMLElement | undefined;

// Draws an art pixel as whole device pixels and puts the image on the device pixel grid, which Chrome
// does not do by itself for an image at a fractional devicePixelRatio.
function fitPup(pup: HTMLElement): void {
  const ratio = window.devicePixelRatio || 1;
  const scale = Math.max(1, Math.ceil(ratio - 0.5));
  const floor = pup.closest<HTMLElement>(".sidebar-pup");
  if (floor) floor.classList.toggle("is-cramped", (75 * scale) / ratio > floorRoom(floor));
  pup.style.setProperty("--pup-px", `calc(${scale} / ${ratio})`);
  const box = pup.getBoundingClientRect();
  const offset = (value: number, round: (x: number) => number) => `${(round(value * ratio) - value * ratio) / ratio}px`;
  pup.style.setProperty("--pup-dx", offset(box.left, Math.round));
  pup.style.setProperty("--pup-dy", offset(box.top, Math.floor));
}

// The width the docs pup may take on the viewport floor. Its edge is 40px left of the content column,
// where the heading rail and demo frames overhang it.
function floorRoom(floor: HTMLElement): number {
  const main = document.querySelector(".main-content");
  if (!main || getComputedStyle(floor).position !== "fixed") return Infinity;
  return main.getBoundingClientRect().left - 40 - floor.getBoundingClientRect().left - 6;
}

function startPups(): void {
  if (pupBody === document.body) return;
  pupBody = document.body;
  const pups = [...document.querySelectorAll<HTMLElement>("[data-pup]")];
  if (pups.length === 0) return;
  const events = new AbortController();
  document.addEventListener("astro:before-swap", () => events.abort(), { once: true });
  const { signal } = events;

  let frame = 0;
  const refit = () => {
    if (!frame)
      frame = window.requestAnimationFrame(() => {
        frame = 0;
        pups.forEach(fitPup);
      });
  };
  pups.forEach(fitPup);
  window.addEventListener("resize", refit, { passive: true, signal });
  // A scroll can stop between device pixels, which shifts the pup off the grid with the page.
  window.addEventListener("scrollend", refit, { passive: true, signal });
  const layout = new ResizeObserver(refit);
  layout.observe(document.body);
  for (const pup of pups) if (pup.parentElement) layout.observe(pup.parentElement);
  void document.fonts?.ready.then(refit);
  signal.addEventListener("abort", () => {
    window.cancelAnimationFrame(frame);
    layout.disconnect();
  });

  for (const pup of pups) {
    pup.addEventListener("click", () => pup.classList.add("is-loved"), { signal });
    pup.addEventListener(
      "animationend",
      (event) => {
        if (event.animationName === "pup-love") pup.classList.remove("is-loved");
      },
      { signal },
    );
  }
}

document.addEventListener("astro:page-load", startPups);
startPups();
