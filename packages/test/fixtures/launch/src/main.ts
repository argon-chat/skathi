import { appLaunch, appShell, createApp, h, ref } from "@skathi-native/runtime";

const opened: string[] = [];
const record = {
  opened,
  // `register` answers instead of throwing, because an exception would come back through eval as a
  // failed command.
  api: {
    register(scheme: string): string {
      try {
        appLaunch.registerProtocol(scheme);
        return "ok";
      } catch (e) {
        const err = e as Error;
        return `${err.name}: ${err.message}`;
      }
    },
    unregister: (scheme: string) => appLaunch.unregisterProtocol(scheme),
    isRegistered: (scheme: string) => appLaunch.isProtocolRegistered(scheme),
    open(url: string): void {
      appShell.openExternal(url).then(
        () => opened.push("ok"),
        (e: Error) => opened.push(`${e.name}: ${e.message}`),
      );
    },
  },
  own: {
    argv: appLaunch.argv,
    cwd: appLaunch.cwd,
    url: appLaunch.url,
    paths: appLaunch.paths,
    singleInstance: appLaunch.singleInstance,
  },
  events: [] as unknown[],
  params: [] as { targetURL: string | null; files: number }[],
};
(globalThis as unknown as { __SK_TEST_LAUNCH__: unknown }).__SK_TEST_LAUNCH__ = record;

const count = ref(0);
appLaunch.addEventListener("launch", (e: SkEvent) => {
  const ev = e as SkEvent & Record<string, unknown>;
  record.events.push({ argv: ev.argv, cwd: ev.cwd, url: ev.url, paths: ev.paths, source: ev.source });
  count.value++;
});
launchQueue.setConsumer((p) => {
  record.params.push({ targetURL: p.targetURL, files: p.files.length });
});

createApp({
  render: () => h("div", { class: "fixture" }, `forwarded launches: ${count.value}`),
}).mount();
