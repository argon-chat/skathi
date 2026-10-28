// A fixture for packages/vn-test/specs/launch.spec.ts (docs/LAUNCH.md).
//
// Records, as plain JSON the devtools `eval` can read back, every launch this
// process hears about: its own (`appLaunch` fields, the first LaunchParams)
// and each forwarded one (the `launch` event and the LaunchParams that
// follows it). A module export is not a global in either boot mode, hence the
// `globalThis` hand-off (see devtools-on/main.ts).
import { appLaunch, appShell, createApp, h, ref } from "vue-native";

const opened: string[] = [];
const record = {
  opened,
  // The tier-2 verbs the spec drives. `register` answers instead of throwing,
  // because an exception would come back through eval as a failed command.
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
(globalThis as unknown as { __VN_TEST_LAUNCH__: unknown }).__VN_TEST_LAUNCH__ = record;

const count = ref(0);
appLaunch.addEventListener("launch", (e: VnEvent) => {
  const ev = e as VnEvent & Record<string, unknown>;
  record.events.push({ argv: ev.argv, cwd: ev.cwd, url: ev.url, paths: ev.paths, source: ev.source });
  count.value++;
});
launchQueue.setConsumer((p) => {
  record.params.push({ targetURL: p.targetURL, files: p.files.length });
});

createApp({
  render: () => h("div", { class: "fixture" }, `forwarded launches: ${count.value}`),
}).mount();
