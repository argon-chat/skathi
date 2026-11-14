// @ts-ignore
export function total(items: any) {
  return items.length as any;
}

export function implicit(x) {
  return x;
}

export const contextual = (xs: number[]) => xs.map((x) => x * 2);

export type Machinery<T> = T extends (...args: any[]) => infer R ? R : never;

export const caught = Promise.resolve().catch((reason) => String(reason));
