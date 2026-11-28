export const answer = await Promise.resolve(42);
/** One object per module instance: an alias that built a second copy would hand out another. */
export const state = { instance: "util.ts" };
export default function greet(who: string): string {
  return `hello ${who}`;
}
