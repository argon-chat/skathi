// Stand-ins for classes redjs defines natively and bun lacks, as far as the runtime's own tests use them.
// Their behaviour is redjs's and tested there.

type Init = EventInit & Record<string, unknown>;

function eventClass(name: string, members: Record<string, unknown>): new (type: string, init?: Init) => Event {
  const own = new WeakMap<object, Record<string, unknown>>();
  const C = class extends Event {
    constructor(type: string, init: Init = {}) {
      super(type, init);
      own.set(this, Object.fromEntries(Object.entries(members).map(([k, d]) => [k, init[k] === undefined ? d : init[k]])));
    }
  };
  Object.defineProperty(C, "name", { value: name });
  for (const k of Object.keys(members)) {
    Object.defineProperty(C.prototype, k, {
      get(this: object) {
        return own.get(this)?.[k];
      },
      enumerable: true,
      configurable: true,
    });
  }
  return C;
}

class DOMRectReadOnly {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
  constructor(x = 0, y = 0, width = 0, height = 0) {
    [this.x, this.y, this.width, this.height] = [x, y, width, height].map(Number) as [number, number, number, number];
  }
  get top(): number {
    return Math.min(this.y, this.y + this.height);
  }
  get left(): number {
    return Math.min(this.x, this.x + this.width);
  }
  get right(): number {
    return Math.max(this.x, this.x + this.width);
  }
  get bottom(): number {
    return Math.max(this.y, this.y + this.height);
  }
}
class DOMRect extends DOMRectReadOnly {}

export const ENGINE_CLASSES: Record<string, unknown> = {
  SubmitEvent: eventClass("SubmitEvent", { submitter: null }),
  ClipboardEvent: eventClass("ClipboardEvent", { clipboardData: null }),
  ErrorEvent: eventClass("ErrorEvent", { message: "", filename: "", lineno: 0, colno: 0, error: null }),
  CompositionEvent: eventClass("CompositionEvent", { data: "" }),
  DOMRectReadOnly,
  DOMRect,
  NodeFilter: Object.freeze({
    SHOW_ALL: 0xffffffff,
    SHOW_ELEMENT: 1,
    SHOW_TEXT: 4,
    SHOW_COMMENT: 128,
    FILTER_ACCEPT: 1,
    FILTER_REJECT: 2,
    FILTER_SKIP: 3,
  }),
};
