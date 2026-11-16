<script setup lang="ts">
import { ref } from "vue";
import { appWindow } from "@skathi-native/runtime";
import Icon from "../components/Icon.vue";
import PopWindow from "../components/PopWindow.vue";
import Scenario from "../components/Scenario.vue";

const opacity = ref(80);
const radius = ref(12);
const closed = ref(false);
const generation = ref(0);

const main = appWindow.getContentBounds();
const bounds = { x: main.x + main.width - 330, y: main.y + 260, width: 280, height: 300 };

function popOutAgain() {
  closed.value = false;
  generation.value++;
}
</script>

<template>
  <Scenario id="pop-out/reattach">
    <div class="card">
      <div class="row">
        <span class="muted state">
          {{
            closed ? "Back inline: the window closed and nothing unmounted it." : "The inspector is in its own window."
          }}
        </span>
        <button v-if="closed" class="btn accent" data-demo="popout-again" @click="popOutAgain">
          <Icon name="reattach" />Pop out again
        </button>
      </div>
    </div>

    <PopWindow
      :key="generation"
      title="Inspector"
      backdrop="acrylic"
      chrome="hidden"
      :bounds="bounds"
      :title-bar-height="36"
      data-demo="inspector-root"
      @close="closed = true"
    >
      <section class="inspector" :class="{ popped: !closed }">
        <header class="bar">
          <span>Inspector</span>
          <span class="close" title="Close" data-demo="inspector-close"><Icon name="close" /></span>
        </header>
        <div class="body">
          <div class="preview" :style="{ opacity: opacity / 100, borderRadius: `${radius}px` }" />
          <div class="field">
            <span class="label">Opacity</span>
            <div class="stepper">
              <button class="step" @click="opacity = Math.max(0, opacity - 10)">−</button>
              <span class="value">{{ opacity }}%</span>
              <button class="step" data-demo="opacity-up" @click="opacity = Math.min(100, opacity + 10)">+</button>
            </div>
          </div>
          <div class="field">
            <span class="label">Corner radius</span>
            <div class="stepper">
              <button class="step" @click="radius = Math.max(0, radius - 4)">−</button>
              <span class="value">{{ radius }} px</span>
              <button class="step" @click="radius = Math.min(40, radius + 4)">+</button>
            </div>
          </div>
        </div>
      </section>
    </PopWindow>
  </Scenario>
</template>

<style scoped>
.state {
  flex: 1;
}
.inspector {
  width: 320px;
  display: flex;
  flex-direction: column;
  border-radius: 8px;
  border: 1px solid var(--card-line);
  background: var(--card);
  color: var(--text);
  font-family: var(--font);
  overflow: hidden;
}
.inspector.popped {
  width: 100%;
  height: 100%;
  border-radius: 0;
  border-width: 0;
  background: var(--popout);
}
.bar {
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-left: 14px;
  font-size: 12px;
  color: var(--text-2);
  app-region: drag;
  flex-shrink: 0;
}
.close {
  width: 40px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-2);
  --window-control: close;
}
.close:hover,
.close.demo-hover {
  background: #c42b1c;
  color: #ffffff;
}
.body {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 8px 16px 16px;
}
.preview {
  height: 96px;
  background: linear-gradient(135deg, #b46fcc, #8a44a3);
  transition:
    opacity 160ms ease,
    border-top-left-radius 160ms ease,
    border-top-right-radius 160ms ease,
    border-bottom-left-radius 160ms ease,
    border-bottom-right-radius 160ms ease;
}
.field {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.label {
  font-size: 13px;
  color: var(--text-2);
}
.stepper {
  display: flex;
  align-items: center;
  gap: 4px;
}
.step {
  width: 30px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 5px;
  border: 1px solid var(--control-line);
  background: var(--control);
  color: var(--text);
  font-size: 14px;
}
.step:hover,
.step.demo-hover {
  background: var(--control-hover);
}
.value {
  width: 56px;
  text-align: center;
  font-size: 13px;
  font-variant-numeric: tabular-nums;
}
</style>
