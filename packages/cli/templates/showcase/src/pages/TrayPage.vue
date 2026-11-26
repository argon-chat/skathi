<script setup lang="ts">
import { defineKeymap, SkKbd } from "@skathi-native/runtime";
import { appHotkeys, appTray } from "skathi:app";
import { ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import ShortcutPrompts from "../components/ShortcutPrompts.vue";
import { describe } from "../engine";
import { useCounter } from "../stores/counter";
import { globalCommand, useDesktop } from "../stores/desktop";

const desktop = useDesktop();
const counter = useCounter();
const recording = ref(false);
const status = ref("");

function toggleGlobal() {
  try {
    desktop.bindHotkey(desktop.boundMap ? null : globalCommand);
    status.value = "";
  } catch (error) {
    status.value = describe(error);
  }
}

// Records the next key combination anywhere on the system; Esc cancels.
async function record() {
  recording.value = true;
  status.value = "Press a combination…";
  try {
    const chord = await appHotkeys.captureOnce();
    desktop.bindHotkey(defineKeymap({ keys: chord, scope: "global", triggerCooldownMs: 150 }));
    status.value = "";
  } catch (e) {
    status.value = describe(e);
  } finally {
    recording.value = false;
  }
}
</script>

<template>
  <Page title="Tray & hotkeys" lead="Desktop integration that keeps working while the window is in the background.">
    <ShortcutPrompts />
    <Card title="Global hotkey" api="defineKeymap">
      <template v-if="appHotkeys.available">
        <div class="row">
          <SkKbd v-if="desktop.boundMap" :map="desktop.boundMap" />
          <span v-else class="muted">Not bound</span>
          <button id="global-keymap-toggle" class="btn" :aria-pressed="!!desktop.boundMap" @click="toggleGlobal">
            {{ desktop.boundMap ? "Unbind" : "Bind" }}
            <SkKbd v-if="!desktop.boundMap" :map="globalCommand" variant="plain" />
          </button>
          <button v-if="!recording" class="btn" @click="record">Record another…</button>
          <button v-else class="btn" @click="appHotkeys.cancelCapture()">Cancel</button>
        </div>
        <p class="muted">Pressed {{ desktop.presses }} times; each press counts up, from any app.</p>
        <p v-if="status" class="faint">{{ status }}</p>
      </template>
      <p v-else class="muted">Global hotkeys need Windows or macOS.</p>
    </Card>

    <Card title="Tray icon" api="appTray.set()">
      <template v-if="appTray.available">
        <div class="row">
          <button class="btn" :class="{ on: desktop.trayShown }" @click="desktop.showTray(!desktop.trayShown)">
            {{ desktop.trayShown ? "Remove the tray icon" : "Show a tray icon" }}
          </button>
        </div>
        <p class="faint">
          Its menu counts up, toggles always-on-top and quits; the label follows the count ({{ counter.count }}). The
          icon is drawn from RGBA pixels, so it works in dev as well as in a build.
        </p>
      </template>
      <p v-else class="muted">This platform has no tray in Skathi yet (Windows and macOS do).</p>
    </Card>
  </Page>
</template>
