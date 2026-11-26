/** HTML's "report an exception": the window's `error` event and `onerror`, then the console unless cancelled. */
export function reportException(error: unknown): void {
  const report = (globalThis as { reportError?: (error: unknown) => void }).reportError;
  if (typeof report === "function") return report(error);
  queueMicrotask(() => {
    throw error;
  });
}
