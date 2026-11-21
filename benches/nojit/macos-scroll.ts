export async function benchmarkScroll(selector: string): Promise<void> {
  const host: typeof globalThis & { __SK_SCENARIO__?: string } = globalThis;
  if (host.__SK_SCENARIO__ !== "macos-scroll") return;

  const frame = () => new Promise<number>((resolve) => requestAnimationFrame(resolve));
  for (let i = 0; i < 20; i++) await frame();
  const element = document.querySelector<HTMLElement>(selector);
  if (!element || element.scrollHeight <= element.clientHeight) {
    console.log(`SKMACSCROLL ${JSON.stringify({ error: `no scrollable element matches ${selector}` })}`);
    return;
  }

  const intervals: number[] = [];
  let moved = 0;
  let direction = 1;
  let previous = await frame();
  for (let i = 0; i < 600; i++) {
    const before = element.scrollTop;
    if (before <= 0) direction = 1;
    else if (before >= element.scrollHeight - element.clientHeight - 1) direction = -1;
    element.scrollTop = before + 37 * direction;
    const now = await frame();
    moved += Math.abs(element.scrollTop - before);
    intervals.push(now - previous);
    previous = now;
  }
  console.log(`SKMACSCROLL ${JSON.stringify({ intervals, moved })}`);
}
