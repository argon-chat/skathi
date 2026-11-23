<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";

type Mode = "wide" | "round" | "hidden";
const modes: { id: Mode; label: string }[] = [
  { id: "wide", label: "Wide" },
  { id: "round", label: "Round" },
  { id: "hidden", label: "display: none" },
];
const mode = ref<Mode>("wide");
const size = ref("");
const hole = ref<HTMLElement | null>(null);

let raf = 0;
function measure() {
  raf = requestAnimationFrame(measure);
  const r = hole.value?.getBoundingClientRect();
  if (r) size.value = `${Math.round(r.width)} × ${Math.round(r.height)}`;
}
onMounted(() => (raf = requestAnimationFrame(measure)));
onBeforeUnmount(() => cancelAnimationFrame(raf));
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">&lt;sk-layer&gt;</span>
      <p>A native layer of the embedder under the page, moved and clipped with its element.</p>
    </div>
    <div class="stage">
      <div ref="hole" class="hole" :class="mode">
        <div class="clip">
          <sk-layer v-show="mode !== 'hidden'" name="plasma" class="layer" />
          <div v-show="mode !== 'hidden'" class="over">
            <div class="top">
              <span class="chip"><span class="live" />native layer</span>
              <span class="chip mono">{{ size }} px</span>
            </div>
            <div class="center">
              <span class="chip"><span class="live" />native layer</span>
            </div>
            <div class="bottom">
              <b>Drawn by the embedder</b>
              <span>Rendered by native code, under the page's text and controls.</span>
            </div>
          </div>
          <div v-if="mode === 'hidden'" class="empty">
            <code>display: none</code>
            <span>The layer is hidden with its element.</span>
          </div>
        </div>
      </div>
    </div>
    <div class="bar">
      <button
        v-for="m in modes"
        :key="m.id"
        class="seg"
        :class="{ on: mode === m.id }"
        :data-demo="m.id"
        @click="mode = m.id"
      >
        {{ m.label }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.head,
.bar {
  position: relative;
  z-index: 1;
}
.hole {
  --ease: cubic-bezier(0.32, 0, 0.18, 1);
  position: relative;
  width: 1000px;
  height: 520px;
  transition:
    width 700ms var(--ease),
    height 700ms var(--ease);
}
.hole.round {
  width: 470px;
  height: 470px;
}
.clip {
  position: absolute;
  inset: 0;
  border-radius: 34px;
  overflow: hidden;
  /* The window's tint, drawn round the box so the layer under it shows untinted. */
  box-shadow:
    0 0 0 2400px var(--tint),
    0 24px 60px #0008;
  transition: border-radius 700ms var(--ease);
}
.round .clip {
  border-radius: 235px;
}
.layer {
  position: absolute;
  inset: 0;
  border-radius: 34px;
  transition: border-radius 700ms var(--ease);
}
.round .layer {
  border-radius: 235px;
}
.over {
  position: absolute;
  inset: 0;
}
.top {
  position: absolute;
  left: 24px;
  right: 24px;
  top: 22px;
  display: flex;
  justify-content: space-between;
}
.center {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 0;
  transition: opacity 120ms ease;
}
.bottom {
  position: absolute;
  left: 30px;
  bottom: 26px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
/* Text leaves before the box changes size and comes back after it. */
.top,
.bottom {
  transition: opacity 200ms ease 550ms;
}
.round .top,
.round .bottom {
  opacity: 0;
  transition: opacity 120ms ease;
}
.round .center {
  opacity: 1;
  transition: opacity 200ms ease 550ms;
}
.bottom b {
  font-family: var(--font-display);
  font-size: 34px;
  font-weight: 600;
  color: #ffffff;
}
.bottom span {
  font-size: 14px;
  color: #ffffffd8;
}
.chip {
  height: 32px;
  padding: 0 14px;
  display: flex;
  align-items: center;
  gap: 8px;
  border-radius: 16px;
  background: #17161eb0;
  color: var(--text);
  font-size: 13px;
}
.mono {
  font-family: var(--mono);
}
.live {
  width: 8px;
  height: 8px;
  border-radius: 4px;
  background: #7fd6a4;
}
.empty {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  border-radius: 34px;
  border: 2px dashed var(--border-strong);
  background: var(--surface);
  color: var(--muted);
}
.empty code {
  font-family: var(--mono);
  font-size: 18px;
  color: var(--lilac);
}
.bar {
  align-self: center;
  display: flex;
  gap: 4px;
  padding: 4px;
  border-radius: 12px;
  background: var(--surface);
  border: 1px solid var(--border);
}
.seg {
  height: 34px;
  padding: 0 18px;
  border: none;
  border-radius: 9px;
  background: transparent;
  color: var(--body);
  font-family: var(--font);
  font-size: 14px;
  transition:
    background-color 160ms ease,
    color 160ms ease;
}
.seg:hover {
  background: #ffffff10;
}
.seg.on {
  background: var(--pink);
  color: var(--ink);
  font-weight: 600;
}
</style>
