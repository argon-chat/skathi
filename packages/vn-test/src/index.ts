// vn-test — Playwright-style end-to-end testing for vue-native apps.
//
//   import { afterAll, test } from "bun:test";
//   import { launch, expect } from "vn-test";
//
//   const app = await launch({ app: "hello" });
//   afterAll(() => app.close());
//
//   test("the Motion tab activates", async () => {
//     await app.locator(".tab:nth-child(3)").click();
//     await expect(app.locator(".tab:nth-child(3)")).toHaveClass("router-link-active");
//   });
//
// `bun test` IS the runner — there is no `vn test` binary and there is not
// going to be one. Everything here is a library that speaks the devtools
// protocol in docs/DEVTOOLS.md, which was shaped for exactly this from the day
// it was written. docs/TESTING.md is the guide, including an honest table of
// what Playwright does that we do not.

export { launch, MIN_BUN, packagedSlot, VnApp, VnCommandError } from "./app";
export type { LaunchOptions, LogEntry, Bounds, NodeInfo, PendingDialog, PendingQuestion, PermissionsInfo } from "./app";
export { Locator, LocatorError, POLL_MS } from "./locator";
export type { LocatorOptions, Resolved } from "./locator";
export { expect, vnExpect, LocatorAssertions } from "./expect";
export type { AssertOptions } from "./expect";
export { flattenStyleDump, styleValue, styleKeys, parseHsla, resolveProperty } from "./styles";
export type { StyleEntry } from "./styles";
export { readPng, changedRegion, inset, Png } from "./pixels";
export type { Rect } from "./pixels";
export { ARTIFACTS_DIR, artifactPath } from "./artifacts";
export { REPO, DEFAULT_EXE, WORK_DIR } from "./dist";
export { findOrphans, reapOrphans } from "./reap";
export type { Orphan } from "./reap";
export { featureExe, featureExeDir, fileContains, packagedExe, bareHost } from "./features";
export type { FeatureExeOptions, PackagedExeOptions } from "./features";
export { uiaTree, uiaSupported, uiaByName, uiaByType, uiaFormat } from "./uia";
export type { UiaNode, UiaOptions } from "./uia";
