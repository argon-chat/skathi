import { section } from "./shared";

export const gamepad = section("skathi:gamepad", [
  {
    slug: "gamepads",
    name: "gamepads",
    kind: "object",
    since: "Unreleased",
    summary: "Read connected controllers, bind physical buttons and sticks, and drive rumble motors.",
    declaration: true,
    notes: [
      "The default host includes the Cargo feature `gamepad`. Input delivery starts on demand and follows window focus. Each controller has an `id` for its current connection and an `index` that a later connection can reuse.",
      "Windows Gaming Input reports changes from its first reading. A control held before the first poll can read as neutral until it moves. The Windows backend keeps polling while a device remains connected, including after the last subscription ends. Linux and macOS retain the device monitor across app reloads.",
      "The standard `buttons` dictionary uses physical positions such as `south`, `left-trigger` and `dpad-up`. Button values range from 0 to 1. The four named stick axes are `left-x`, `left-y`, `right-x` and `right-y`. Up and left are negative. Use `buttonValues` and `axisValues` for numeric input or an unmapped device.",
      "Both standard sticks use the same radial dead zone. Supply a finite radius from 0 up to (excluding) 1. For example, 0.08 suppresses the inner 8% of stick travel and rescales the remaining travel continuously. Zero passes the axes through. The native and web APIs share the calibrated values. Buttons, triggers and unmapped axes keep their original values.",
      "The host checks the device override first, then the model, then the global rule, then the backend default. Pass null to remove one override. getDeadZone(target) returns the explicit rule for that target or null. Read controller.deadZone from a fresh list() snapshot for the effective radius. Changes apply to already connected controllers, including a held stick, and to future connections.",
      "Model targets take the reported hardware vendorId and productId, including the IDs reported by a virtual controller. The vendorId must be nonzero. Known USB and Bluetooth variants of the Xbox Elite Wireless Controller Series 2 share a model rule. Device targets take an opaque systemId or the current numeric controller.id. A systemId can be configured before the device connects. With a Puck it identifies a radio slot on that receiver. If the OS cannot identify the endpoint unambiguously, systemId is null and a rule set by id lasts for that connection.",
      "Overrides belong to the current app session. An app that remembers calibration across launches can save its target and radius in its own preferences and reapply them at startup.",
      "Use `list()` for devices already present and `subscribe()` for changes. Subscriptions receive `connected`, `disconnected`, `button` and `axis` events. Button events carry a value, a pressed flag and an `edge` of `down`, `up` or `null` for pressure changes within the same state. Additional unmapped controls receive a numeric slot on first input and keep it for the current connection.",
      "Pass the controller to the `controller` prop of `SkKbd` for model-specific legends. Xbox and PlayStation mappings use the same physical positions. Xbox Elite controllers with a model-specific hardware ID retain their model name, including the Series 2 over Bluetooth.",
      "On Windows, a standalone app can read the Steam Controller (2026) directly through its wireless Puck. A controller appears when its radio slot reports a connection or input. The default radius is 4% for the Puck and zero for other input backends. Beyond the standard layout, button indices 17 through 25 cover Quick Access, R4, L4, R5, L5, right and left pad clicks, then left and right trigger clicks. This connection supports input only.",
      "Windows apps launched in a Steam game session use the game’s Steam Input layout. Choose a layout with gamepad output to expose a virtual controller. Its identity and legends follow the emulated device. Other Steam Controller connections rely on the OS driver or Steam Input. Steamworks action sets and glyph lookup require a separate Steamworks integration.",
      "Rumble uses two strengths from 0 to 1. The defaults are 1 for both motors and 120 ms. The maximum duration is 5,000 ms. Pass zero duration to stop. Check `canRumble` and the return value. The gilrs macOS backend supports input only.",
    ],
    examples: [
      {
        title: "Calibrate all controllers, a model or one device",
        lang: "ts",
        code: `import { gamepads } from "skathi:gamepad";

gamepads.setDeadZone({ scope: "global" }, 0.05);
gamepads.setDeadZone({ scope: "model", vendorId: 0x045e, productId: 0x0b22 }, 0.08);

const pad = gamepads.list()[0];
if (pad?.systemId) {
  const target = { scope: "device", systemId: pad.systemId } as const;
  gamepads.setDeadZone(target, 0.12);
  gamepads.setDeadZone(target, null); // Inherit the model or global rule again.
}`,
      },
      {
        title: "Accept with the south face button",
        lang: "ts",
        code: `import { gamepads } from "skathi:gamepad";

const unsubscribe = gamepads.subscribe(event => {
  if (event.type !== "button" || event.button !== "south" || event.edge !== "down") return;
  acceptSelection();
  if (event.gamepad.canRumble) {
    gamepads.rumble(event.gamepad.id, { weak: 0.35, strong: 0, durationMs: 60 });
  }
});
window.addEventListener("pagehide", unsubscribe, { once: true });`,
      },
    ],
  },
]);
