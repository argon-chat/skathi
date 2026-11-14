<script setup lang="ts">
import { appHotkeys, appTray } from "@skathi-native/runtime";
import { ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { describe } from "../engine";
import { useCounter } from "../stores/counter";
import { chordLabel, DEFAULT_CHORD, useDesktop } from "../stores/desktop";

const desktop = useDesktop();
const counter = useCounter();
const recording = ref(false);
const status = ref("");

// Records the next key combination anywhere on the system; Esc cancels.
async function record() {
  recording.value = true;
  status.value = "Press a combination…";
  try {
    const chord = await appHotkeys.captureOnce();
    desktop.bindHotkey(chord);
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

    <Card title="Global hotkey" api="appHotkeys">
      <template v-if="appHotkeys.available">
        <div class="row">
          <span class="pill" :class="{ ok: desktop.chord }">
            {{ desktop.chord ? chordLabel(desktop.chord) : "not bound" }}
          </span>
          <button class="btn" @click="desktop.bindHotkey(desktop.chord ? null : DEFAULT_CHORD)">
            {{ desktop.chord ? "Unbind" : `Bind ${chordLabel(DEFAULT_CHORD)}` }}
          </button>
          <button v-if="!recording" class="btn" @click="record">Record another…</button>
          <button v-else class="btn" @click="appHotkeys.cancelCapture()">Cancel</button>
        </div>
        <p class="muted">Pressed {{ desktop.presses }} times; each press counts up, from any app.</p>
        <p v-if="status" class="faint">{{ status }}</p>
      </template>
      <p v-else class="muted">Global hotkeys need Windows or macOS.</p>
    </Card>
  </Page>
</template>
