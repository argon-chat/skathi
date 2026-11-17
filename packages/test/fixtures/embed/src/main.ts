import { appHost, createApp, h, ref } from "@skathi-native/runtime";
import { available, checksum, crash, double, on, point, refuse, repeat, report, reversed } from "native:demo";
import { uiThread, worker, type Value } from "native:demo";
import { sum } from "native:extra";

const status = ref("starting");

function attempt(f: () => unknown): unknown {
  try {
    return { ok: f() };
  } catch (e) {
    const err = e as Error;
    return { error: err.name, message: err.message };
  }
}

async function settle(p: Promise<unknown>): Promise<unknown> {
  try {
    return { ok: await p };
  } catch (e) {
    const err = e as Error & { function?: string };
    return { error: err.name, message: err.message, function: err.function ?? null };
  }
}

function shown(bytes: Uint8Array): string {
  return `${bytes.constructor.name}:${bytes.join()}`;
}

async function run(): Promise<void> {
  const seen = {
    available,
    point: attempt(() => point(2, 3)),
    refuse: attempt(() => refuse()),
    badArgument: attempt(() => point("two" as unknown as number, 3)),
    uiThread: uiThread(),
    sum: sum(2, 3),
    double: await settle(double(21)),
    doubleZero: await settle(double(0)),
    crash: await settle(crash()),
    worker: await settle(worker()),
    legacy: await settle(appHost.call("ping", "hello")),
    bytes: {
      checksum: checksum(new Uint8Array([1, 2, 3])),
      checksumBuffer: checksum(new Uint8Array([4, 5]).buffer),
      checksumArray: checksum([6, 7] as unknown as Uint8Array),
      reversed: shown(reversed(new Uint8Array([1, 2, 3]))),
      repeated: shown(await repeat(new Uint8Array([7, 8]), 2)),
    },
  };
  on("tick", (tick) => {
    status.value = "ticked";
    report({ tick });
  });
  appHost.on("legacy", (data) => report({ legacy: data as Value }));
  status.value = "reported";
  report(seen as unknown as Value);
}

createApp({
  setup() {
    void run();
    return () => h("div", { id: "app", style: { padding: "12px" } }, [h("div", { id: "status" }, status.value)]);
  },
}).mount();
