import { afterEach, expect, test } from "bun:test";
import { createRenderer, h, nextTick, ref } from "@vue/runtime-core";
import { SkWindow } from "../src/skwindow";

const globals = globalThis as Record<string, unknown>;
const original = new Map<string, unknown>();
const set = (name: string, value: unknown) => {
  original.set(name, globals[name]);
  globals[name] = value;
};
afterEach(() => {
  for (const [name, value] of original) {
    if (value === undefined) delete globals[name];
    else globals[name] = value;
  }
  original.clear();
});

test("SkWindow applies live and pre-open bounds without remounting or activating", async () => {
  const opened: unknown[] = [];
  const changed: Array<Record<string, unknown>> = [];
  const closed: number[] = [];
  let event: (kind: string, data: string) => void = () => {};
  set("__sk_window_open", (id: number, options: string, _close: unknown, onEvent: typeof event) => {
    opened.push([id, JSON.parse(options)]);
    event = onEvent;
  });
  set("__sk_window_set_options", (_id: number, options: string) => changed.push(JSON.parse(options)));
  set("__sk_window_close", (id: number) => closed.push(id));
  let id = 0;
  const renderer = createRenderer<any, any>({
    createElement: () => ({ __skId: ++id, children: [] }),
    createText: () => ({}),
    createComment: () => ({}),
    setText() {},
    setElementText() {},
    patchProp() {},
    remove() {},
    insert: (node, parent) => parent.children.push(node),
    parentNode: () => null,
    nextSibling: () => null,
  });
  const bounds = ref({ x: 10, y: 20, width: 300, height: 180 });
  const app = renderer.createApp({
    render: () => h(SkWindow, { bounds: bounds.value, activate: false, contentProtection: true }),
  });
  app.mount({ children: [] });
  expect(opened).toHaveLength(1);
  bounds.value.x = 30;
  await nextTick();
  expect(changed).toHaveLength(0);
  event("open", '{"windowId":7}');
  expect(changed.at(-1)?.x).toBe(30);
  bounds.value = { x: 80, y: 50, width: 420, height: 250 };
  await nextTick();
  expect(changed.at(-1)).toMatchObject({
    x: 80,
    y: 50,
    width: 420,
    height: 250,
    activate: false,
    contentProtection: true,
  });
  expect(opened).toHaveLength(1);
  expect(closed).toHaveLength(0);
  app.unmount();
  expect(closed).toHaveLength(1);
});
