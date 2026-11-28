<script setup lang="ts">
import {
  SkContextMenu,
  SkKbd,
  SkMenuItem,
  SkMenuSeparator,
  SkSubmenu,
  type SkContextMenuHandle,
  type SkKbdKeys,
} from "@skathi-native/runtime";
import { ref, useTemplateRef } from "vue";
import GamepadView from "./GamepadView.vue";

type Shot = "keys" | "keyboard" | "layouts" | "mouse" | "gamepad" | "menu" | "pad";
const shot = ref<Shot>("keys");
const menu = useTemplateRef<SkContextMenuHandle<string>>("menu");

const rows: Record<
  Exclude<Shot, "menu" | "pad">,
  readonly (readonly [string, SkKbdKeys, Record<string, string>?])[]
> = {
  keys: [
    ["Comment line", { sequence: ["Mod+K", "Mod+C"] }],
    ["Quit", ["Mod", { code: "KeyQ" }]],
    ["Zoom", ["Mod", { mouse: "wheel-up" }]],
    ["Confirm", { alternatives: ["Enter", { gamepad: "south" }] }, { gamepadLayout: "xbox" }],
    ["Aim and fire", [{ gamepad: "left-trigger" }, { gamepad: "right-trigger" }], { gamepadLayout: "xbox" }],
  ],
  keyboard: [
    ["Command palette", "Mod+Shift+P"],
    ["Comment line", { sequence: ["Mod+K", "Mod+C"] }],
    ["Zoom in", { alternatives: [["Mod", "+"], "Mod+NumpadAdd"] }],
    ["Find", "Mod+F", { platform: "macos" }],
  ],
  layouts: [
    ["Quit", ["Mod", { code: "KeyQ" }]],
    ["Quit on macOS", ["Mod", { code: "KeyQ" }], { platform: "macos" }],
  ],
  mouse: [
    ["Select", { mouse: "primary" }],
    ["Context menu", { mouse: "secondary" }],
    ["Go back", { mouse: "back" }],
    ["Zoom", ["Mod", { mouse: "wheel-up" }]],
  ],
  gamepad: [
    ["Confirm", { alternatives: ["Enter", { gamepad: "south" }] }, { gamepadLayout: "xbox" }],
    ["Cancel", { gamepad: "east" }, { gamepadLayout: "playstation" }],
    ["Aim and fire", [{ gamepad: "left-trigger" }, { gamepad: "right-trigger" }], { gamepadLayout: "xbox" }],
    ["Next item", { gamepad: "dpad-right" }],
  ],
};

function show(id: Shot) {
  menu.value?.close();
  shot.value = id;
  return "ok";
}
function openMenu() {
  const row = document.querySelector(".file");
  if (!row) return "no row";
  const r = row.getBoundingClientRect();
  row.dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      button: 2,
      clientX: r.left + 150,
      clientY: r.top + r.height / 2,
    }),
  );
  return "ok";
}
Object.assign(globalThis, { __shots: { show, openMenu } });
</script>

<template>
  <main>
    <GamepadView v-if="shot === 'pad'" />
    <ul v-else-if="shot === 'menu'" class="list">
      <li
        v-for="file in ['Release notes.md', 'Cover artwork.png', 'App icon.svg']"
        :key="file"
        class="row file"
        @contextmenu.prevent="menu?.open($event, file)"
      >
        {{ file }}
      </li>
    </ul>
    <ul v-else class="list">
      <li v-for="[name, keys, props] in rows[shot]" :key="name" class="row">
        <span>{{ name }}</span>
        <SkKbd :keys="keys" v-bind="props" />
      </li>
    </ul>
    <SkContextMenu ref="menu" v-slot="{ context: file }" aria-label="File actions" debug-aim>
      <SkMenuItem>Open {{ file }}</SkMenuItem>
      <SkMenuItem disabled>Rename</SkMenuItem>
      <SkMenuSeparator />
      <SkSubmenu label="Move to">
        <SkMenuItem>Drafts</SkMenuItem>
        <SkMenuItem>Review</SkMenuItem>
        <SkMenuItem>Archive</SkMenuItem>
      </SkSubmenu>
      <SkMenuItem>Copy file name</SkMenuItem>
    </SkContextMenu>
  </main>
</template>

<style>
html,
body {
  margin: 0;
  background: transparent;
  color: #f2eef7;
  font: 15px/1.4 system-ui;
}
main {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 100vh;
  padding: 24px;
  box-sizing: border-box;
}
.list {
  display: flex;
  flex-direction: column;
  width: 100%;
  margin: 0;
  padding: 0;
  list-style: none;
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 52px;
  padding: 0 4px;
  border-bottom: 1px solid rgb(255 255 255 / 0.08);
}
.row .sk-kbd {
  font-size: 15px;
}
.row:last-child {
  border-bottom: 0;
}
</style>
