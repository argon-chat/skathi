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
