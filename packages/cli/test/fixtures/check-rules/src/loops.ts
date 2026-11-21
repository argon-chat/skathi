export function bound(values: Int32Array, n: number): number {
  let total = 0;
  for (let i = 0; i < n; i++) total = (total + values[i]!) | 0;
  return total;
}

export function countdown(left: number): number {
  let steps = 0;
  while (left > steps) steps = Math.imul(steps + 1, 3);
  return steps;
}

export function stores(out: number[], registers: number[], by: number): void {
  for (let i = 0; i < out.length; i++) {
    out[i] = (i * by) | 0;
    registers[i & 31] = out[i]! + 1;
  }
}

// Nothing below may be reported.
export function integerBound(values: Int32Array, n: number): number {
  const end = n | 0;
  let total = 0;
  for (let i = 0; i < end; i++) total = (total + values[i]!) | 0;
  return total;
}

export function calls(out: number[], n: number, next: () => number): void {
  for (let i = 0; i < n; i++) out[i] = next();
}

export function typed(out: Int32Array, names: string[], count?: number): void {
  for (let i = 0; i < out.length; i++) out[i] = i;
  for (let i = 0; i < (count ?? 0); i++) names[i] = "";
  const fixed: number[] = [0, 0];
  for (let i = 0; i < 4; i++) fixed[0] = i;
}
