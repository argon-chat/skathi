<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import { appWindow } from "@skathi-native/runtime";
import Scenario from "../components/Scenario.vue";
import { titleBar } from "../store";

const dark = matchMedia("(prefers-color-scheme: dark)");

function paintButtons() {
  appWindow.setTitleBarOverlay({ symbolColor: dark.matches ? "#ffffffe0" : "#1b1b1be0" });
}

const DOCS: Record<string, { title: string; lines: string[] }> = {
  Notes: {
    title: "Release notes",
    lines: [
      "Pop-out windows keep their place when an unrelated prop changes.",
      "The fallback title bar retires on the first painted drag region.",
      "Host caption buttons follow the theme through symbolColor.",
    ],
  },
  Canvas: {
    title: "Poster draft",
    lines: ["Three shapes, one ink color.", "The palette pop-out edits the ink live.", "Export at 2x for the docs."],
  },
  Export: {
    title: "Export settings",
    lines: ["Format: WebM (AV1) and MP4 (H.264).", "Width: 1280 px, no audio.", "Poster: one WebP frame per clip."],
  },
};

onMounted(() => {
  titleBar.tabs = true;
  if (navigator.platform !== "MacIntel") {
    paintButtons();
    dark.addEventListener("change", paintButtons);
  }
});
onBeforeUnmount(() => {
  dark.removeEventListener("change", paintButtons);
  titleBar.tabs = false;
  appWindow.setTitleBarOverlay(false);
});
</script>

<template>
  <Scenario id="title-bar/overlay">
    <div class="card doc">
      <span class="tab-name">{{ titleBar.tab }}</span>
      <h2 class="doc-title">{{ DOCS[titleBar.tab]?.title }}</h2>
      <ul class="lines">
        <li v-for="line in DOCS[titleBar.tab]?.lines" :key="line">{{ line }}</li>
      </ul>
    </div>
    <div class="card reads">
      <div class="read">
        <span class="k">chrome.hostControls</span><span class="v">{{ appWindow.chrome.hostControls }}</span>
      </div>
      <div class="read">
        <span class="k">chrome.titleBarHeight</span><span class="v">{{ appWindow.chrome.titleBarHeight }} px</span>
      </div>
      <p class="muted">
        Hover the maximize button for the Windows 11 snap layouts. The hover colors are fixed by the host, which is why
        the gallery draws its own buttons everywhere else.
      </p>
    </div>
  </Scenario>
</template>

<style scoped>
.tab-name {
  font-size: 12px;
  color: var(--text-3);
}
.doc-title {
  margin: 0;
  font-family: var(--font-display);
  font-size: 20px;
  font-weight: 600;
}
.lines {
  margin: 0;
  padding-left: 20px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 14px;
  line-height: 21px;
  color: var(--text-2);
}
.reads {
  gap: 10px;
}
.read {
  display: flex;
  align-items: center;
  gap: 16px;
}
.k {
  width: 200px;
  font-family: var(--mono);
  font-size: 12px;
  color: var(--text-3);
}
.v {
  font-size: 14px;
  color: var(--text);
}
</style>
