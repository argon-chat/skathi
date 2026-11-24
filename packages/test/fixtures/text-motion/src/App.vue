<script setup lang="ts">
import { nextTick, ref } from "vue";

const page = ref("rest");
const moved = ref(false);
const offsets = Array.from({ length: 10 }, (_, k) => k / 10);
const baselines = Array.from({ length: 10 }, () => "H H H H H H H H").join("\n");
const lines = Array.from({ length: 40 }, (_, i) => `H l H l H l H l H ${i}`).join("\n");

(globalThis as { __SK_TEST_MOTION__?: unknown }).__SK_TEST_MOTION__ = {
  show(name: string) {
    page.value = name;
  },
  scroll(top: number) {
    const el = document.getElementById("scroller");
    if (el) el.scrollTop = top;
  },
  transition() {
    return document.startViewTransition(() => {
      moved.value = !moved.value;
      return nextTick();
    }).ready;
  },
};
</script>

<template>
  <div class="root">
    <div v-if="page === 'rest'" class="rest">
      <div
        v-for="(o, k) in offsets"
        :key="k"
        class="at"
        :style="{ top: `${16 + k * 30}px`, letterSpacing: `${o / 10}px` }"
      >
        l l l l l l l l l l
      </div>
      <div class="baselines">{{ baselines }}</div>
    </div>
    <div v-else-if="page === 'motion'" class="motion">
      <div class="lane"><div class="stems pulse">l l l l l l l l l l l l</div></div>
      <div class="lane"><div class="stems slide">l l l l l l l l l l l l</div></div>
      <div class="lane"><div class="stems slide layer">l l l l l l l l l l l l</div></div>
      <div class="lane"><div class="stems spin">l l l l l l l l l l l l</div></div>
    </div>
    <div v-else-if="page === 'scroll'" id="scroller" class="scroller">
      <div class="lines">{{ lines }}</div>
    </div>
    <div v-else-if="page === 'vt'" class="vt">
      <div :class="['stems', 'card', { moved }]">l l l l l l l l l l l l</div>
    </div>
  </div>
</template>

<style>
.root {
  position: absolute;
  inset: 0;
  background: #fff;
  color: #111;
  font-family: "Segoe UI", sans-serif;
}
.at {
  position: absolute;
  left: 20px;
  font-size: 16px;
  line-height: 20px;
  white-space: pre;
}
.baselines {
  position: absolute;
  left: 300px;
  top: 16px;
  font-size: 16px;
  line-height: 20.3px;
  white-space: pre;
}
.motion {
  padding: 24px;
}
.lane {
  height: 70px;
}
.stems {
  display: inline-block;
  font-size: 16px;
  line-height: 20px;
  white-space: pre;
  transform-origin: 0 0;
}
.pulse {
  animation: pulse 1.3s linear infinite alternate;
}
.slide {
  animation: slide 2.3s linear infinite alternate;
}
.layer {
  will-change: transform;
}
.spin {
  animation: spin 1.7s linear infinite alternate;
}
@keyframes pulse {
  from {
    transform: scale(1);
  }
  to {
    transform: scale(1.3);
  }
}
@keyframes slide {
  from {
    transform: translate(0, 0);
  }
  to {
    transform: translate(9.3px, 3.7px);
  }
}
@keyframes spin {
  from {
    transform: translate(0.3px, 0.2px) scale(1.05);
  }
  to {
    transform: translate(5.1px, 2.9px) scale(1.12);
  }
}
.scroller {
  position: absolute;
  left: 24px;
  top: 24px;
  width: 400px;
  height: 300px;
  overflow-y: auto;
}
.lines {
  font-size: 13px;
  line-height: 17px;
  white-space: pre;
}
.card {
  position: absolute;
  left: 40px;
  top: 40px;
  view-transition-name: stems;
}
.card.moved {
  left: 340px;
  top: 203px;
}
::view-transition-group(stems) {
  animation-duration: 4s;
  animation-timing-function: linear;
}
</style>
