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

/** Turns the devtools off at runtime or back on; `SK_DEVTOOLS=1` holds them on and refuses `disable()`. */
export interface AppDevtools {
  /** False only for `--features no-devtools`; unlike `isEnabled()`, "not in the binary" cannot be reversed. */
  readonly available: boolean;
  isEnabled(): boolean;
  /** Binds a fresh port and token and republishes `<dist>/devtools.json`; idempotent, so an attached inspector keeps its port. */
  enable(): boolean;
  /** Closes the listener and inspector sockets, blanks the token, deletes the handshake. Returns the state afterwards. */
  disable(): boolean;
  /** Programmatic F12. */
  open(): boolean;
  info(): DevtoolsInfo;
  onChange(fn: (enabled: boolean) => void): () => void;
}

declare const __sk_appDevtools: AppDevtools;
export const appDevtools: AppDevtools = __sk_appDevtools;
