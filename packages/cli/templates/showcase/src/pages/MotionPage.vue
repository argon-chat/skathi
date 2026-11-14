<script setup lang="ts">
import { ref, shallowRef } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";

const expanded = ref(false);

const target = shallowRef<HTMLElement | null>(null);
const shake = () =>
  target.value?.animate(
    [
      { transform: "translateX(0px)" },
      { transform: "translateX(-10px)" },
      { transform: "translateX(10px)" },
      { transform: "translateX(0px)" },
    ],
    { duration: 360, easing: "ease-in-out", iterations: 2 },
  );
const spin = () =>
  target.value?.animate([{ transform: "rotate(0deg) scale(1)" }, { transform: "rotate(360deg) scale(1.15)" }], {
    duration: 700,
    easing: "cubic-bezier(0.34, 1.56, 0.64, 1)",
  });

const shown = ref(true);
const words = ["Native", "Fast", "Small", "Vue"];
const word = ref(0);
const nextWord = () => (word.value = (word.value + 1) % words.length);
</script>

<template>
  <Page
    title="Motion"
    lead="CSS transitions and keyframes run in the renderer; the Web Animations API drives them from script."
  >
    <Card title="Transition" api="transition">
      <div class="track" @click="expanded = !expanded">
        <div class="thumb" :class="{ expanded }">{{ expanded ? "wide" : "narrow" }} · click</div>
      </div>
    </Card>

    <Card title="Keyframes" api="@keyframes">
      <div class="row gap">
        <div class="pulse" />
        <div class="orbit"><div class="moon" /></div>
        <div class="shimmer">Loading…</div>
      </div>
    </Card>

    <Card title="Web Animations" api="element.animate()">
      <div class="row gap">
        <div ref="target" class="box">✦</div>
        <button class="btn" @click="shake">Shake</button>
        <button class="btn" @click="spin">Spin</button>
      </div>
    </Card>

    <Card title="Vue transitions" api="<Transition>">
      <div class="row">
        <button class="btn" @click="shown = !shown">{{ shown ? "Hide" : "Show" }} the panel</button>
        <button class="btn" @click="nextWord">Next word</button>
      </div>
      <Transition name="fade">
        <div v-if="shown" class="panel">Enter and leave classes, as in a browser.</div>
      </Transition>
      <Transition name="swap" mode="out-in">
        <div :key="word" class="word">{{ words[word] }}</div>
      </Transition>
    </Card>
  </Page>
</template>

<style scoped>
.gap {
  gap: 20px;
}
.track {
  height: 44px;
  padding: 4px;
  border-radius: 12px;
  background: #00000040;
  cursor: pointer;
}
.thumb {
  width: 160px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 9px;
  background: var(--surface-strong);
  font-size: 12px;
  color: var(--text-muted);
  transition:
    width 420ms cubic-bezier(0.34, 1.56, 0.64, 1),
    background-color 420ms ease,
    color 420ms ease;
}
.thumb.expanded {
  width: 420px;
  background: var(--accent-strong);
  color: #ffffff;
}
@keyframes pulse {
  0% {
    transform: scale(0.8);
    opacity: 1;
  }
  100% {
    transform: scale(1.6);
    opacity: 0;
  }
}
.pulse {
  width: 28px;
  height: 28px;
  border-radius: 14px;
  background: var(--accent);
  animation: pulse 1.4s ease-out infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.orbit {
  width: 48px;
  height: 48px;
  border-radius: 24px;
  border: 1px dashed var(--line-strong);
  animation: spin 3s linear infinite;
}
.moon {
  width: 12px;
  height: 12px;
  margin-left: 18px;
  margin-top: -6px;
  border-radius: 6px;
  background: #b48cff;
}
@keyframes shimmer {
  50% {
    opacity: 0.35;
  }
}
.shimmer {
  padding: 8px 14px;
  border-radius: 8px;
  background: var(--surface-strong);
  font-size: 12px;
  animation: shimmer 1.6s ease-in-out infinite;
}
.box {
  width: 52px;
  height: 52px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 14px;
  background: linear-gradient(135deg, var(--accent), #b48cff);
  color: #ffffff;
  font-size: 22px;
}
.panel {
  padding: 14px;
  border-radius: 10px;
  background: var(--accent-soft);
  font-size: 13px;
}
.fade-enter-active,
.fade-leave-active {
  transition:
    opacity 250ms ease,
    transform 250ms ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
  transform: translateY(-6px);
}
.word {
  font-size: 28px;
  font-weight: 700;
}
.swap-enter-active,
.swap-leave-active {
  transition:
    opacity 180ms ease,
    transform 180ms ease;
}
.swap-enter-from {
  opacity: 0;
  transform: translateY(10px);
}
.swap-leave-to {
  opacity: 0;
  transform: translateY(-10px);
}
</style>
