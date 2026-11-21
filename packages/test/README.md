# @skathi-native/test

End-to-end tests for [Skathi](https://skathi.land) apps, as a library for `bun test`. It builds your app, starts it in the dev host `sk dev` uses, and drives the real window through Playwright-style locators and assertions.

```bash
bun add -d @skathi-native/test @types/bun
```

Keep the tests outside `src/`, with a tsconfig of their own that has `"types": ["bun"]`: the app's code runs in Skathi, not in Bun.

```ts
import { afterAll, test } from "bun:test";
import { expect, launch } from "@skathi-native/test";

const app = await launch({ app: "." });
afterAll(() => app.close());

test("the counter counts", async () => {
  await app.locator("button.count").click();
  await expect(app.locator("button.count")).toHaveText("count is 1");
});
```

`app` is the app's directory or its entry module, relative to where `bun test` runs. Build output and host logs go to `.sk-test/`, failure screenshots to `.sk-test-artifacts/`. Use the version that matches `@skathi-native/cli`.
