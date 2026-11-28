import type { SkKbdGamepadButton, SkKbdGamepadLayout } from "../kbd-devices";

export type ControllerFamily = Exclude<SkKbdGamepadLayout, "dualshock4">;
export type ControllerButton = SkKbdGamepadButton;
export type ControllerAxis = "left-x" | "left-y" | "right-x" | "right-y";

/** A model uses its USB vendor/product pair (vendor must be nonzero). A current connection id
 * addresses its systemId when available, otherwise the setting expires when the device disconnects. */
export type DeadZoneTarget =
  | { scope: "global" }
  | { scope: "model"; vendorId: number; productId: number }
  | { scope: "device"; systemId: string }
  | { scope: "device"; id: number };

export interface Controller {
  /** Identifies a single connection. Use this for rumble and per-player state. */
  readonly id: number;
  /** Opaque system identifier for this device, when the backend can distinguish it across connections. */
  readonly systemId: string | null;
  readonly index: number;
  readonly name: string;
  readonly family: ControllerFamily;
  readonly vendorId: number;
  readonly productId: number;
  readonly mapping: "" | "standard";
  readonly timestamp: number;
  readonly canRumble: boolean;
  /** Effective stick dead-zone radius for this device, including any inherited setting. */
  readonly deadZone: number;
  /** Physical positions have values from 0 to 1. */
  readonly buttons: Readonly<Partial<Record<ControllerButton, number>>>;
  /** Stick axes range from -1 to 1. Up is negative. */
  readonly axes: Readonly<Partial<Record<ControllerAxis, number>>>;
  readonly buttonValues: readonly number[];
  readonly axisValues: readonly number[];
}

export type ControllerEvent =
  | { readonly type: "connected" | "disconnected"; readonly gamepad: Controller }
  | {
      readonly type: "button";
      readonly gamepad: Controller;
      readonly index: number;
      readonly button: ControllerButton | null;
      readonly value: number;
      readonly pressed: boolean;
      readonly edge: "down" | "up" | null;
    }
  | {
      readonly type: "axis";
      readonly gamepad: Controller;
      readonly index: number;
      readonly axis: ControllerAxis | null;
      readonly value: number;
    };

export interface Gamepads {
  /** Whether the host includes controller support. */
  readonly available: boolean;
  list(): readonly Controller[];
  subscribe(listener: (event: ControllerEvent) => void): () => void;
  /** Returns just this target's override, or null when it inherits its setting. */
  getDeadZone(target: DeadZoneTarget): number | null;
  /** Set a finite radius in [0, 1). Zero disables the stick dead zone. Pass null to inherit. */
  setDeadZone(target: DeadZoneTarget, radius: number | null): void;
  /** Motor strengths are 0..1. A duration of zero stops the effect. */
  rumble(id: number, options?: { strong?: number; weak?: number; durationMs?: number }): boolean;
}

declare const __sk_std: { gamepads?: Gamepads };

export const gamepads: Gamepads = __sk_std.gamepads ?? {
  available: false,
  list: () => [],
  subscribe: () => () => {},
  getDeadZone: () => null,
  setDeadZone: () => {},
  rumble: () => false,
};
