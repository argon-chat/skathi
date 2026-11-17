#!/usr/bin/env bun
import { fileURLToPath } from "node:url";

const sk = fileURLToPath(import.meta.resolve("@skathi-native/cli/cli/sk.ts"));
const child = Bun.spawn([process.execPath, sk, "create", ...process.argv.slice(2)], {
  stdio: ["inherit", "inherit", "inherit"],
});
process.exit(await child.exited);
