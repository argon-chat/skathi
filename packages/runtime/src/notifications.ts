export interface NotificationCapabilities {
  readonly available: boolean;
  readonly management: "none" | "session" | "application";
  readonly activation: "none" | "running" | "relaunch";
  readonly maxActions: number;
  readonly silent: boolean;
  readonly inlineReply: boolean;
  readonly image: boolean;
  readonly grouping: boolean;
}

export type NotificationAction =
  { kind: "button"; id: string; label: string } | { kind: "reply"; id: string; label: string };

export interface DesktopNotificationOptions {
  title: string;
  body?: string;
  id?: string;
  group?: { id: string; label: string };
  image?: { asset: string } | { png: Uint8Array };
  sound?: "default" | "silent";
  actions?: readonly NotificationAction[];
}

export interface NotificationReceipt {
  readonly id: string;
  readonly submittedActions: readonly string[];
  readonly omitted: readonly ("body" | "group" | "image")[];
}

export type NotificationActivation =
  | { kind: "open"; id: string }
  | { kind: "button"; id: string; actionId: string }
  | { kind: "reply"; id: string; actionId: string; text: string };

export interface AppNotifications {
  capabilities(): Promise<NotificationCapabilities>;
  /** Reusing an ID replaces the card. A receipt acknowledges native submission. */
  show(options: DesktopNotificationOptions): Promise<NotificationReceipt>;
  close(id: string): Promise<void>;
  /** Activations received before registration are delivered in arrival order. */
  setActivationHandler(handler: ((event: NotificationActivation) => void) | null): void;
}

declare const __sk_appNotifications: AppNotifications;
export const appNotifications: AppNotifications = __sk_appNotifications;
