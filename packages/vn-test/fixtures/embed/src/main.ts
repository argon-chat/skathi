// A fixture for packages/vn-test/specs/embed.spec.ts: the app inside
// crates/vn-app/examples/embed.rs, a program that runs vue-native as a library
// (docs/EMBEDDING.md).
//
// On mount it goes through what an embedded app does with `appHost` and hands
// the embedder what it saw with `appHost.call("report", …)`:
// - `available` and `functions`;
// - `ping`, answered on the call, and `add`, answered later from another
//   thread of the embedder's;
// - `fail`, which rejects, and `missing`, which the embedder does not have;
// - the `tick` event the embedder emits after the first report, and its echo
//   back as a second report.
//
// Render functions rather than an SFC, for the reason fixtures/layout gives.
import { appHost, createApp, h, ref } from "vue-native";

const status = ref("starting");

async function settle(p: Promise<unknown>): Promise<unknown> {
  try {
    return { ok: await p };
  } catch (e) {
    const err = e as Error & { function?: string };
    return { error: err.name, message: err.message, function: err.function ?? null };
  }
}

async function run(): Promise<void> {
  const seen = {
    available: appHost.available,
    functions: appHost.functions,
    ping: await settle(appHost.call("ping", "hello")),
    add: await settle(appHost.call("add", 2, 3)),
    fail: await settle(appHost.call("fail")),
    missing: await settle(appHost.call("missing")),
  };
  appHost.on("tick", (data) => {
    status.value = "ticked";
    void appHost.call("report", { tick: data });
  });
  status.value = "reported";
  await appHost.call("report", seen);
}

createApp({
  setup() {
    void run();
    return () => h("div", { id: "app", style: { padding: "12px" } }, [h("div", { id: "status" }, status.value)]);
  },
}).mount();
