import { defineKeymap } from "@skathi-native/runtime";

export function bindPreview(actions: { isOpen(): boolean; select(): void; open(): void; showDetails(): void }) {
  const shortcut = defineKeymap({
    keys: { alternatives: ["F2", { mouse: "back" }] },
    when: () => actions.isOpen(),
    doubleClickMs: 400,
    holdMs: 600,
  });
  const off = [
    shortcut.on("click", () => actions.select()),
    shortcut.on("doubleClick", () => actions.open()),
    shortcut.on("hold", () => actions.showDetails()),
  ];
  return {
    shortcut,
    pause: shortcut.disable,
    resume: shortcut.enable,
    dispose: () => off.forEach((unsubscribe) => unsubscribe()),
  };
}
