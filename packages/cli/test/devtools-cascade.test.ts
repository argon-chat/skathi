import { describe, expect, test } from "bun:test";
import { overriddenDeclarations } from "../../../apps/devtools/src/cascade";
import type { Declaration, Rule } from "../../../apps/devtools/src/protocol";

const rule = (declarations: Declaration[], over: Partial<Rule> = {}): Rule => ({
  selector: ".x",
  specificity: 10,
  order: 0,
  state: "base",
  declarations,
  ...over,
});
const decl = (property: string, value = "1px", important = false): Declaration => ({
  property,
  value,
  ...(important ? { important } : {}),
});
const dead = (rules: Rule[]) => [...overriddenDeclarations(rules)].sort();

describe("the Styles pane's overridden declarations", () => {
  test("a later declaration of the same property wins", () => {
    expect(dead([rule([decl("color")]), rule([decl("color")])])).toEqual(["0:0"]);
  });

  test("a longhand overrides only part of an earlier shorthand, a later shorthand all of a longhand", () => {
    expect(dead([rule([decl("padding")]), rule([decl("padding-left")])])).toEqual([]);
    expect(dead([rule([decl("padding-left")]), rule([decl("padding")])])).toEqual(["0:0"]);
    expect(
      dead([
        rule([decl("margin")]),
        rule([decl("margin-top"), decl("margin-right"), decl("margin-bottom"), decl("margin-left")]),
      ]),
    ).toEqual(["0:0"]);
    expect(dead([rule([decl("border-color")]), rule([decl("border")])])).toEqual(["0:0"]);
  });

  test("!important beats a later normal declaration, longhand or shorthand", () => {
    expect(dead([rule([decl("color", "red", true)]), rule([decl("color")])])).toEqual(["1:0"]);
    expect(dead([rule([decl("padding-top", "1px", true)]), rule([decl("padding")])])).toEqual([]);
    expect(dead([rule([decl("padding", "1px", true)]), rule([decl("padding-top")])])).toEqual(["1:0"]);
    expect(dead([rule([decl("color", "red", true)]), rule([decl("color", "blue", true)])])).toEqual(["0:0"]);
  });

  test("a pseudo-element's rules and an inactive state's rules do not override the element's", () => {
    expect(dead([rule([decl("--tw-x")]), rule([decl("--tw-x")], { pseudo: "after" })])).toEqual([]);
    expect(dead([rule([decl("color")]), rule([decl("color")], { state: "hover" })])).toEqual([]);
  });
});
