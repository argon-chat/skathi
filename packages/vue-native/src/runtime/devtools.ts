import { warnCompat } from "../compat";

// `appDevtools` lets an app turn the devtools off at runtime (`disable()`, or `"devtools": false` in vn.config.json):
// the listener closes, inspectors detach, the token is blanked and the handshake file deleted; `enable()` binds a
// fresh port and token. `VN_DEVTOOLS=1` forces them on and refuses `disable()`.

interface DevtoolsHost {
  state: typeof __vn_devtools_state;
  setEnabled: typeof __vn_devtools_set_enabled;
  open: typeof __vn_devtools_open;
}

export interface DevtoolsInfo {
  enabled: boolean;
  locked: boolean;
  port: number | null;
  pid: number;
  key: boolean;
  inspectors: number;
  /** The dist slot whose `devtools.json` this host publishes into. */
  slot: string;
}

interface AppDevtools {
  /** False only for `--features no-devtools`; unlike `isEnabled()`, "not in the binary" cannot be reversed. */
  readonly available: boolean;
  isEnabled(): boolean;
  enable(): boolean;
  disable(): boolean;
  open(): boolean;
  info(): DevtoolsInfo;
  onChange(fn: (enabled: boolean) => void): () => void;
}

const devtoolsHost: DevtoolsHost | null =
  typeof __vn_devtools_state === "function"
    ? {
        state: __vn_devtools_state,
        setEnabled: __vn_devtools_set_enabled,
        open: __vn_devtools_open,
      }
    : null;

const devtoolsWatchers = new Set<(enabled: boolean) => void>();

function notifyDevtools(enabled: boolean) {
  for (const fn of [...devtoolsWatchers]) {
    try {
      fn(enabled);
    } catch (e) {
      // One listener's throw must not stop the others or fail a successful `disable()`.
      warnCompat("appDevtools.onChange", `a change listener threw and was ignored: ${String(e)}`);
    }
  }
}

function createDevtools(host: DevtoolsHost): AppDevtools {
  const read = (): DevtoolsInfo => JSON.parse(host.state()) as DevtoolsInfo;
  const set = (on: boolean): boolean => {
    const was = read().enabled;
    const out = JSON.parse(host.setEnabled(on)) as DevtoolsInfo & { error?: string };
    if (out.error) {
      // Refused (only by `VN_DEVTOOLS=1`), not failed: warn rather than throw so a shipped app's boot survives.
      warnCompat("appDevtools", `${on ? "enable" : "disable"}() was refused: ${out.error}`);
      return was;
    }
    if (out.enabled !== was) notifyDevtools(out.enabled);
    return out.enabled;
  };
  return {
    available: true,
    isEnabled: () => read().enabled,
    /** Binds a fresh port and token and republishes `<dist>/devtools.json`; idempotent, so an attached inspector keeps its port. */
    enable: () => set(true),
    /** Closes the listener and inspector sockets, blanks the token, deletes the handshake. Returns the state afterwards: `true` means `VN_DEVTOOLS=1` overruled it. */
    disable: () => set(false),
    open: () => {
      const out = JSON.parse(host.open()) as { opened?: boolean; error?: string };
      if (out.error) {
        warnCompat("appDevtools", `open() did not open an inspector: ${out.error}`);
        return false;
      }
      return !!out.opened;
    },
    info: () => read(),
    onChange(fn: (enabled: boolean) => void) {
      devtoolsWatchers.add(fn);
      return () => {
        devtoolsWatchers.delete(fn);
      };
    },
  };
}

/** `--features no-devtools`: nothing to switch; answers instead of throwing so `disable()` in an entry module is harmless. */
const inertDevtools: AppDevtools = {
  available: false,
  isEnabled: () => false,
  enable: () => {
    warnCompat(
      "appDevtools",
      "this host was built with `--features no-devtools`, so there are no devtools to enable " +
        "— `appDevtools.enable()` did nothing.",
    );
    return false;
  },
  disable: () => false,
  open: () => {
    warnCompat(
      "appDevtools",
      "this host was built with `--features no-devtools` — `appDevtools.open()` has no " + "inspector to open.",
    );
    return false;
  },
  info: () => ({
    enabled: false,
    locked: false,
    port: null,
    pid: 0,
    key: false,
    inspectors: 0,
    slot: "",
  }),
  onChange(fn: (enabled: boolean) => void) {
    devtoolsWatchers.add(fn);
    return () => {
      devtoolsWatchers.delete(fn);
    };
  },
};

export const appDevtools: AppDevtools = devtoolsHost ? createDevtools(devtoolsHost) : inertDevtools;
