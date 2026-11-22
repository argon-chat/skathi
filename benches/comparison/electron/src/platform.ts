type Native = {
  add(a: number, b: number): Promise<number>;
  len(bytes: Uint8Array): Promise<number>;
  bench(): Promise<boolean>;
};
const native = (window as unknown as { native: Native }).native;

export function signal(state: string) {
  document.title = `Compare ${state}`;
}
export const benchMode = () => native.bench();
export const add = (a: number, b: number) => native.add(a, b);
export const len = (bytes: Uint8Array) => native.len(bytes);
