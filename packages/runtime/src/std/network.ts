// `skathi:network`: what the OS knows about the connection. The host installs the object.

export type ConnectionType = "ethernet" | "wifi" | "cellular" | "other" | "none" | "unknown";
export type Internet = "full" | "limited" | "portal" | "none" | "unknown";

interface NetworkEvents {
  /** Any property changed; also fired after `online` and `offline`. */
  change: Event;
  online: Event;
  offline: Event;
}

export interface Network extends EventTarget {
  /** `navigator.onLine`: the OS believes there is a network, not that the internet is reachable. */
  readonly online: boolean;
  /** The OS's own connectivity check; Skathi sends no probe of its own. */
  readonly internet: Internet;
  readonly type: ConnectionType;
  /** The connection is billed by use. `null`: the OS cannot tell. */
  readonly metered: boolean | null;
  readonly roaming: boolean | null;
  readonly overDataLimit: boolean | null;
  addEventListener<K extends keyof NetworkEvents>(
    type: K,
    listener: (event: NetworkEvents[K]) => void,
    options?: boolean | AddEventListenerOptions,
  ): void;
  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
}

declare const __SK_NETWORK__: Network;

export const network: Network = __SK_NETWORK__;
