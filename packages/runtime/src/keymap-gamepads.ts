import type { ControllerEvent, gamepads } from "./std/gamepad";

type ButtonChange = Extract<ControllerEvent, { type: "button" }>;
export type KeymapGamepadInput = CustomEvent<ButtonChange>;

export function isGamepadInput(event: Event): event is KeymapGamepadInput {
  return event.type === "skgamepadbuttondown" || event.type === "skgamepadbuttonup";
}

export function gamepadHost(): typeof gamepads | undefined {
  const host = (globalThis as { __sk_std?: { gamepads?: typeof gamepads } }).__sk_std?.gamepads;
  return host?.available ? host : undefined;
}

export function watchGamepadButtons(
  host: typeof gamepads,
  connected: (id: number) => void,
  disconnected: (id: number) => void,
) {
  const devices = host.list();
  const stop = host.subscribe((event) => {
    const pad = event.gamepad;
    if (event.type === "connected") return connected(pad.id);
    if (event.type === "disconnected") return disconnected(pad.id);
    if (event.type !== "button" || !event.edge) return;
    const target = (document as Document & { activeElement?: EventTarget | null }).activeElement ?? document;
    target.dispatchEvent(
      new CustomEvent("skgamepadbutton" + event.edge, {
        detail: event,
        bubbles: true,
        cancelable: true,
      }),
    );
  });
  for (const pad of devices) connected(pad.id);
  return stop;
}
