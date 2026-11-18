import { expect, test } from "bun:test";
import { parseKeys } from "../cli/prompt";

test("arrow keys in both encodings, and a lone Escape cancels", () => {
  expect(parseKeys("\x1b[A\x1bOB\x1b[C\x1b[D").map((k) => k.name)).toEqual(["up", "down", "right", "left"]);
  expect(parseKeys("\x1b")).toEqual([{ name: "cancel" }]);
});

test("a paste is its characters, Delete and Home are ignored, Ctrl+C cancels", () => {
  expect(parseKeys("my app\r")).toEqual([
    { name: "char", char: "m" },
    { name: "char", char: "y" },
    { name: "space" },
    { name: "char", char: "a" },
    { name: "char", char: "p" },
    { name: "char", char: "p" },
    { name: "enter" },
  ]);
  expect(parseKeys("\x1b[3~\x1b[1;5H")).toEqual([]);
  expect(parseKeys("\x03")).toEqual([{ name: "cancel" }]);
});
