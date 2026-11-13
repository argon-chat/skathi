import { expect, test } from "bun:test";
import { installShare } from "../src/share";

class FakeDOMException extends Error {
  constructor(message?: string, name?: string) {
    super(message);
    this.name = name ?? "Error";
  }
}

function setup(answer: () => Promise<unknown> = () => Promise.resolve({})) {
  const sent: { req: Record<string, unknown>; sizes: number[] }[] = [];
  let active = true;
  const nav = {} as { share(d?: unknown): Promise<void>; canShare(d?: unknown): boolean };
  installShare(nav, {
    host: {
      share: (json, buffers) => {
        sent.push({ req: JSON.parse(json), sizes: buffers.map((b) => b.byteLength) });
        return '{"token":1}';
      },
    },
    call: answer,
    resolveUrl: (u) => new URL(u, "app://redjs/index.html").href,
    isFile: (v): v is File => v instanceof File,
    activation: { isActive: () => active, consume: () => void (active = false) },
    documentTitle: () => "Doc",
    DOMException: FakeDOMException,
  });
  return { nav, sent, activate: () => void (active = true) };
}

test("a share sends the fields, the files and consumes the activation", async () => {
  const { nav, sent } = setup();
  await nav.share({ text: "hi", url: "/x", files: [new File(["abc"], "a.txt")] });
  expect(sent).toEqual([{ req: { title: "Doc", text: "hi", url: "app://redjs/x", files: ["a.txt"] }, sizes: [3] }]);
  await expect(nav.share({ text: "again" })).rejects.toMatchObject({ name: "NotAllowedError" });
});

test("dismissing the sheet is an AbortError; one share at a time", async () => {
  let reject: (f: unknown) => void = () => {};
  const { nav, activate } = setup(() => new Promise((_, r) => (reject = r)));
  const first = nav.share({ title: "t" });
  activate();
  await expect(nav.share({ title: "t" })).rejects.toMatchObject({ name: "InvalidStateError" });
  reject({ name: "AbortError", message: "Share canceled" });
  await expect(first).rejects.toMatchObject({ name: "AbortError", message: "Share canceled" });
});

test("canShare says what share would refuse", () => {
  const { nav } = setup();
  expect(nav.canShare({ title: "t" })).toBe(true);
  expect(nav.canShare({})).toBe(false);
  expect(nav.canShare()).toBe(false);
  expect(nav.canShare({ url: "http://[" })).toBe(false);
  expect(nav.canShare({ files: ["not a file"] })).toBe(false);
  expect(nav.canShare({ files: [] })).toBe(false);
});
