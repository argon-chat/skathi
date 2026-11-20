import { NO, OK, importLine, os, section } from "./shared";

export const haptics = section("Haptics", [
  {
    slug: "app-haptics",
    name: "appHaptics",
    kind: "interface",
    since: "0.6.0",
    summary: "Play tactile feedback on a pointing device or stop a pen's continuous stroke feedback.",
    platforms: os(OK, OK, NO),
    syntax: `${importLine("appHaptics")}

readonly appHaptics.supported: boolean
appHaptics.supportsPointer(pointerId: number): boolean
appHaptics.perform(pattern: HapticPattern, options?: { pointerId?: number; when?: "now" | "next-frame" }): boolean
appHaptics.stop(pointerId: number): boolean`,
    notes: [
      'Use "alignment" for snapping, "level-change" for a slider step and "generic" for a confirmation. Call `appHaptics.perform("generic")` from a button to send an immediate pulse to the current device, including a compatible mouse or trackpad. Pass a PointerEvent’s `pointerId` when the feedback should follow that event’s device.',
      "`true` means the platform accepted an immediate request or queued an effect for the next presented frame. The OS and device settings control whether it can be felt. An unavailable device, an unknown pointer or an unsupported effect returns `false`.",
      'On macOS, AppKit plays the three discrete patterns on a Force Touch trackpad, including Magic Trackpad 2. AppKit has no actuator query, so support becomes true after the first native pressure gesture in the session. "next-frame" uses AppKit’s draw-completed timing.',
      'On Windows 11, mouse and touchpad feedback requires the InputHapticsManager runtime and a compatible device. For "generic", Skathi prefers the Hover waveform on mice and touchpads and uses Click on pens. With "next-frame", the latest cue for each device waits for a successful DirectX present.',
      'Windows pens such as Surface Slim Pen 2 also require a compatible digitizer. Pass the in-range pen’s pointer ID. During contact, "ink-continuous" and "pencil-continuous" start a stroke texture. Pencil falls back to ink when necessary. Release, leaving range, loss of focus or closing the window stops continuous feedback.',
      "Call `stop(pointerId)` to end continuous feedback early or cancel a queued Windows cue. It returns true when it cancels or stops something. On Linux the namespace is present and returns false.",
    ],
    examples: [
      {
        title: "Feel a grid snap",
        lang: "ts",
        code: `import { appHaptics } from "@skathi-native/runtime";

function onSnap(event: PointerEvent) {
  appHaptics.perform("alignment", { pointerId: event.pointerId, when: "next-frame" });
}`,
      },
    ],
    aliases: [
      "Force Touch",
      "haptics",
      "Slim Pen",
      "appHaptics.perform",
      "appHaptics.stop",
      "appHaptics.supportsPointer",
      "appHaptics.supported",
    ],
  },
]);
