/** Calls `draw` once per painted frame until the returned function is called. */
export function everyFrame(draw: (time: number) => void): () => void {
  let handle = requestAnimationFrame(function frame(time) {
    draw(time);
    handle = requestAnimationFrame(frame);
  });
  return () => cancelAnimationFrame(handle);
}

export const describe = (e: unknown) => (e instanceof Error ? e.message : String(e));
