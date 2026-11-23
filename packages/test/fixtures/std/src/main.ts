import { appShell as fromRuntime, createApp, h } from "@skathi-native/runtime";
import { app, appShell } from "skathi:app";
import { network } from "skathi:network";
import { u8 } from "skathi:numeric";
import { OS, os } from "skathi:os";
import { Runtime, runtime } from "skathi:runtime";
import { TOML } from "skathi:toml";
import { binary, encoding } from "./bytes";
import { time, units } from "./clock";
import { EXPECTED, gpu } from "./gpu";
import { image } from "./image";
import { memory, memoryWorkers } from "./memory";
import { connection, connectionChange } from "./network";
import { callbacks, ffi, structs, threads, workers } from "./native";
import { flood, lingering, processes } from "./process";
import { collections } from "./structures";
import { diffs, diffsAsync, text } from "./strings";

const DOCUMENT = `title = "Skathi"
big = 9007199254740993
ratio = 0.5
when = 1979-05-27T07:32:00Z
day = 1979-05-27
tags = ["a", "b"]

[window]
width = 800
resizable = true

[[servers]]
name = "one"
`;

function toml() {
  const parsed = TOML.parse(DOCUMENT) as Record<string, unknown> & { window: Record<string, unknown> };
  const kind = (value: unknown) => Object.prototype.toString.call(value).slice(8, -1);
  return {
    keys: Object.keys(parsed),
    title: parsed.title,
    big: String(parsed.big),
    bigType: typeof parsed.big,
    ratio: parsed.ratio,
    when: [kind(parsed.when), (parsed.when as Date).toISOString()],
    day: [kind(parsed.day), String(parsed.day)],
    window: parsed.window,
    servers: parsed.servers,
    round: TOML.parse(TOML.stringify(parsed)).big === parsed.big,
    written: TOML.stringify({
      name: "x",
      skipped: undefined,
      n: 1n << 62n,
      at: new Date(0),
      list: [1, 2],
      t: { ok: true },
    }),
    syntax: attempt(() => TOML.parse('a = 1\nb = "unclosed\n')),
    nil: attempt(() => TOML.stringify({ window: { title: null } })),
    hole: attempt(() => TOML.stringify({ list: [1, undefined] })),
    notTable: attempt(() => TOML.stringify([1] as unknown as Record<string, unknown>)),
  };
}

const attempt = (run: () => void): string => {
  try {
    run();
    return "ok";
  } catch (e) {
    const error = e as Error;
    return `${error.name}: ${error.message}`;
  }
};

Object.assign(globalThis, {
  __SK_TEST_STD__: {
    app,
    toml: () => JSON.stringify(toml()),
    binary: () => JSON.stringify(binary()),
    encoding: () => JSON.stringify(encoding()),
    units: () => JSON.stringify(units()),
    collections: () => JSON.stringify(collections()),
    text: () => JSON.stringify(text()),
    diffs: () => JSON.stringify(diffs()),
    diffsResult: null as string | null,
    startDiffs() {
      void diffsAsync().then(
        (result) => (this.diffsResult = JSON.stringify({ result })),
        (error: unknown) => (this.diffsResult = JSON.stringify({ failed: String(error) })),
      );
    },
    ffi: () => JSON.stringify(ffi()),
    callbacks: () => JSON.stringify(callbacks()),
    memory: () => JSON.stringify(memory(), (_, v: unknown) => (typeof v === "bigint" ? String(v) : v)),
    memoryWorkersResult: null as string | null,
    startMemoryWorkers() {
      void memoryWorkers().then(
        (result) => (this.memoryWorkersResult = JSON.stringify({ result })),
        (error: unknown) => (this.memoryWorkersResult = JSON.stringify({ failed: String(error) })),
      );
    },
    imageResult: null as string | null,
    startImage() {
      void image().then(
        (result) => (this.imageResult = JSON.stringify({ result })),
        (error: unknown) => (this.imageResult = JSON.stringify({ failed: String(error) })),
      );
    },
    gpuResult: null as string | null,
    startGpu() {
      void gpu().then(
        (result) => (this.gpuResult = JSON.stringify({ ...result, expected: EXPECTED })),
        (error: unknown) => (this.gpuResult = JSON.stringify({ failed: String(error) })),
      );
    },
    workersResult: null as string | null,
    structsResult: null as string | null,
    startStructs() {
      void structs().then(
        (result) => (this.structsResult = JSON.stringify({ result })),
        (error: unknown) => (this.structsResult = JSON.stringify({ failed: String(error) })),
      );
    },
    startWorkers() {
      void workers().then(
        (result) => (this.workersResult = JSON.stringify({ result })),
        (error: unknown) => (this.workersResult = JSON.stringify({ failed: String(error) })),
      );
    },
    threadsResult: null as string | null,
    startThreads() {
      void threads().then(
        (result) => (this.threadsResult = JSON.stringify({ result })),
        (error: unknown) => (this.threadsResult = JSON.stringify({ failed: String(error) })),
      );
    },
    processResult: null as string | null,
    startProcess() {
      void processes().then(
        (result) => (this.processResult = JSON.stringify({ result })),
        (error: unknown) => (this.processResult = JSON.stringify({ failed: String(error) })),
      );
    },
    floodResult: null as string | null,
    startFlood(size: number) {
      void flood(size).then(
        (length) => (this.floodResult = JSON.stringify({ length })),
        (error: unknown) => (this.floodResult = JSON.stringify({ failed: String(error) })),
      );
    },
    lingering: () => lingering(),
    timeResult: null as string | null,
    startTime() {
      void time().then(
        (result) => (this.timeResult = JSON.stringify(result)),
        (error: unknown) => (this.timeResult = JSON.stringify({ failed: String(error) })),
      );
    },
    connectionResult: null as string | null,
    startConnectionChange() {
      void connectionChange().then(
        (result) => (this.connectionResult = JSON.stringify(result)),
        (error: unknown) => (this.connectionResult = JSON.stringify({ failed: String(error) })),
      );
    },
    network: () =>
      JSON.stringify({
        online: network.online,
        sameOnline: network.online === navigator.onLine,
        internet: network.internet,
        type: network.type,
        metered: network.metered,
        roaming: network.roaming,
        overDataLimit: network.overDataLimit,
        target: network instanceof EventTarget,
        connection: connection(),
        wrapped: u8(257),
      }),
    probe: () =>
      JSON.stringify({
        version: app.getVersion(),
        sameShell: appShell === fromRuntime,
        os: {
          platform: os.platform,
          arch: os.arch,
          machineArch: os.machineArch,
          release: os.release,
          distro: os.distro,
          hostname: os.hostname,
          homedir: os.homedir,
          tmpdir: os.tmpdir,
          isOS: os instanceof (OS as unknown as new () => object),
          frozenDistro: os.distro === null || Object.isFrozen(os.distro),
        },
        runtime: {
          version: runtime.version,
          features: [...runtime.features],
          isAot: Runtime.isAot(),
          isSingleFile: Runtime.isSingleFile(),
        },
        badExitCode: attempt(() => app.quit({ exitCode: 1.5 })),
        badArgs: attempt(() => app.restart({ args: [1 as unknown as string] })),
      }),
  },
});

createApp({ render: () => h("div", { id: "ready" }, `std ${app.getVersion()}`) }).mount();
