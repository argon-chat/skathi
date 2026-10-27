// A stand-in for a whole vn-test run, for `orphan.spec.ts` — NOT a spec.
//
// Launches one host, prints `RUNNER <pid> HOST <pid>` on stdout, and then hangs
// until something kills it — which is the point: the spec kills this process
// outright, so no `close()`, exit hook or signal handler of ours ever runs.
//
//   bun packages/vn-test/specs/_orphan-runner.ts [keep]
//
// `keep` launches with `keepAlive`, i.e. without `VN_OWNER_PID`: a host that
// SHOULD outlive its runner, standing in for one the owner watch cannot reach.

import { launch } from "../src/index";

const keepAlive = process.argv.includes("keep");
const app = await launch({ app: "hello", window: [640, 480], label: "orphan", keepAlive });
console.log(`RUNNER ${process.pid} HOST ${app.pid} SLOT ${app.slotDir}`);
await new Promise(() => {});
