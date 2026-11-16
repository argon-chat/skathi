<script setup lang="ts">
import { computed } from "vue";
import { appWindow } from "@skathi-native/runtime";
import Icon from "../components/Icon.vue";
import Scenario from "../components/Scenario.vue";

function toggleFullscreen() {
  if (document.fullscreenElement) void document.exitFullscreen();
  else void document.documentElement.requestFullscreen();
}

const full = computed(() => appWindow.isFullscreen());
</script>

<template>
  <Scenario id="windows/fullscreen">
    <div class="stage card">
      <div class="slide">
        <span class="kicker">Quarterly review</span>
        <span class="headline">Three releases, one window model</span>
        <span class="sub">Pop-outs, overlays and custom chrome, all from one Vue tree</span>
      </div>
      <div class="controls">
        <button class="btn accent" data-demo="fullscreen" @click="toggleFullscreen">
          <Icon name="fullscreen" />{{ full ? "Leave full screen" : "Full screen" }}
        </button>
        <span class="hint">{{ full ? "Press Esc to leave." : "Esc leaves full screen too." }}</span>
      </div>
    </div>

    <div class="card reads">
      <div class="read">
        <span class="k">isFullscreen()</span><span class="v">{{ full }}</span>
      </div>
      <div class="read">
        <span class="k">chrome.displayMode</span><span class="v">{{ appWindow.chrome.displayMode }}</span>
      </div>
    </div>
  </Scenario>
</template>

<style scoped>
.stage {
  padding: 0;
  overflow: hidden;
}
.slide {
  height: 260px;
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  gap: 8px;
  padding: 32px;
  background: linear-gradient(135deg, #3b1a49, #8a44a3 55%, #d79fe8);
  color: #ffffff;
}
@media (display-mode: fullscreen) {
  .slide {
    height: 420px;
  }
}
.kicker {
  font-size: 12px;
  font-weight: 600;
  color: #ffffffb0;
}
.headline {
  font-family: var(--font-display);
  font-size: 32px;
  font-weight: 600;
}
.sub {
  font-size: 14px;
  color: #ffffffd0;
}
.controls {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 16px 20px;
}
.hint {
  font-size: 13px;
  color: var(--text-3);
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
