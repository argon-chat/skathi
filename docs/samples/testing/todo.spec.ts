// #region setup
import { afterAll, describe, setDefaultTimeout, test } from "bun:test";
import { expect, launch } from "@skathi-native/test";

setDefaultTimeout(30_000);

const app = await launch({
  app: "docs/samples/testing/todo/src/main.ts",
  label: "todo",
  window: [480, 360],
});
afterAll(() => app.close());
// #endregion setup

describe("todos", () => {
  // #region first
  test("adds a todo from the keyboard", async () => {
    const field = app.locator(".new-todo");
    await field.type("Buy milk");
    await field.press("Enter");

    await expect(app.locator(".todo")).toHaveCount(1);
    await expect(app.locator(".todo").first()).toHaveText("Buy milk");
    await expect(field).toHaveText("");
    await expect(app.locator(".count")).toHaveText("1 item left");
  });
  // #endregion first

  // #region click
  test("clicking a todo marks it done", async () => {
    await app.locator(".new-todo").type("Walk the dog");
    await app.locator(".add").click();
    await expect(app.locator(".todo")).toHaveCount(2);

    const first = app.locator(".todo").first();
    await first.click();

    await expect(first).toHaveClass("done");
    await expect(app.locator(".todo").nth(1)).not.toHaveClass("done");
    await expect(app.locator(".count")).toHaveText("1 item left");
  });
  // #endregion click

  // #region eval
  test("the window title follows the count", async () => {
    expect(await app.eval("document.title")).toBe("Todos (1)");
  });
  // #endregion eval

  // #region errors
  test("the app logged no errors", async () => {
    await app.expectNoErrors();
  });
  // #endregion errors
});
