import { appShell as fromRuntime, createApp, h } from "@skathi-native/runtime";
import { app, appShell } from "skathi:app";
import { OS, os } from "skathi:os";
import { Runtime, runtime } from "skathi:runtime";

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
