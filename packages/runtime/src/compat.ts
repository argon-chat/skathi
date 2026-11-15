// One-shot compat advisories, in the host's set: never silently drop behavior a browser has; say what to use instead.

export function hasWarned(key: string): boolean {
  return __sk_advised(key);
}

export function warnCompat(key: string, msg: string): void {
  __sk_advise(key, msg);
}
