// One-shot compat advisories: never silently drop behavior a browser has; say what to use instead.
const warned = new Set<string>();

export function hasWarned(key: string): boolean {
  return warned.has(key);
}

export function warnCompat(key: string, msg: string): void {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(`[vue-native compat] ${msg}`);
}
