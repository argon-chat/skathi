// Mirrors sit on HTMLElement.prototype so `instanceof HTMLElement`/`Node` checks in reused browser components pass.
// The bun test harness has no HTMLElement; a private prototype stands in so the DOM surface never lands on Object.prototype.
export const ElementProto: object =
  typeof HTMLElement === "function" ? HTMLElement.prototype : Object.create(Object.prototype);
export const TextProto: object = Object.create(Object.prototype);
/** `Node.prototype` once interfaces.ts installs `Node`: it carries the Node members too, so `Object.getOwnPropertyDescriptor(Node.prototype, "firstChild")` answers as in a browser. */
export const NodeProto: object = Object.create(Object.prototype);

// A mirror node must never become reactive state: ours stringify as `[object Object]`, which Vue's `getTargetType`
// would Proxy-wrap, so `set.has`, `indexOf` and `===` would disagree about one node (reka-ui's layer stack and focus
// guards compare identity). `__v_skip` (markRaw) on the prototypes stops it.
for (const proto of [ElementProto, TextProto]) {
  Object.defineProperty(proto, "__v_skip", { value: true, configurable: true });
}

export const NODE_MEMBERS: object[] = [ElementProto, TextProto, NodeProto];
