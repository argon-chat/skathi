import { expect, test } from "bun:test";
import { readClassSpecs, rustBinding } from "../scripts/rust-bindings";

test("indexes registered prototypes, resolves names and ignores test-only classes", () => {
  const source = `
    const NAME: &str = "Dial";
    fn install() {
      cx.register(ClassSpec {
        name: NAME, methods: &[("rotate", 1, rotate)], getters: &[("angle", angle)], accessors: &[],
      });
      let unused = ClassSpec { name: "Unused", methods: &[] };
    }
    #[cfg(test)]
    mod tests { fn fake() { cx.register(ClassSpec { name: "Mock", methods: &[] }); } }
  `;
  expect(readClassSpecs(source, "dial.rs")).toEqual([
    {
      name: "Dial",
      source: "dial.rs",
      members: [
        { name: "rotate", kind: "method", arity: 1 },
        { name: "angle", kind: "getter" },
      ],
    },
  ]);
});

test("reads rustdoc on named specs only when passed to registration", () => {
  const source = `
    fn install() { support::class(g, &DIAL); define_class(g, &self::DIAL); }
    /// A dial.
    #[allow(dead_code)]
    /// Calling \`rotate()\` advances its angle.
    const DIAL: ClassSpec<'static> = ClassSpec { name: "Dial", methods: &[("rotate", 1, rotate)] };
    /// Internal dummy.
    const UNUSED: ClassSpec<'static> = ClassSpec { name: "Unused", methods: &[] };
    fn unused() { inspect(&UNUSED); }
    #[allow(dead_code)]
    #[cfg(test)]
    fn test_only() { register(&UNUSED); }
  `;
  expect(readClassSpecs(source, "dial.rs")).toEqual([
    {
      name: "Dial",
      source: "dial.rs",
      documentation: "A dial.\nCalling \`rotate()\` advances its angle.",
      members: [{ name: "rotate", kind: "method", arity: 1 }],
    },
  ]);
});

test("keeps std classes out of web globals and follows redjs registrations", () => {
  expect(rustBinding("web", "TimeSpan")).toBeUndefined();
  expect(rustBinding("skathi:time", "TimeSpan")?.members).toContainEqual({ name: "add", kind: "method", arity: 1 });
  expect(rustBinding("skathi:time", "TimeSpan")?.documentation).toContain("`toJSON()` return milliseconds");
  expect(rustBinding("web", "ReadableStream")?.members).toContainEqual({ name: "locked", kind: "getter" });
  expect(rustBinding("web", "History")?.members).toContainEqual({ name: "scrollRestoration", kind: "accessor" });
});
