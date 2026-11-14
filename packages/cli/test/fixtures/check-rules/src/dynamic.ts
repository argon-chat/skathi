export const direct = () => eval("1 + 1");
export const indirect = () => (0, eval)("2");
export const viaGlobal = () => globalThis.eval("3");
export const constructed = () => new Function("a", "return a");
export const called = () => Function("return 1");
export const timer = () => setInterval("tick()", 10);
export const computed = (name: string) => import(name);
