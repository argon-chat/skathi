<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const ACCENTS = [
  { name: "pink", color: "#f7a8d0", second: "#c9a7f5" },
  { name: "lilac", color: "#c9a7f5", second: "#9fcdf5" },
  { name: "blue", color: "#9fcdf5", second: "#c9a7f5" },
];
const frame = ref<HTMLIFrameElement | null>(null);
const accent = ref("pink");
const toast = ref("");
let hideTimer = 0;

function pick(a: (typeof ACCENTS)[number]) {
  accent.value = a.name;
  frame.value?.contentWindow?.postMessage({ accent: a.color, second: a.second }, "/");
}

function onMessage(event: Event) {
  const e = event as Event & { source?: unknown; data?: unknown };
  if (e.source !== frame.value?.contentWindow) return;
  const applied = (e.data as { applied?: string })?.applied;
  if (!applied) return;
  toast.value = applied;
  clearTimeout(hideTimer);
  hideTimer = setTimeout(() => (toast.value = ""), 1500) as unknown as number;
}
onMounted(() => window.addEventListener("message", onMessage));
onUnmounted(() => window.removeEventListener("message", onMessage));
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">&lt;iframe&gt;</span>
      <p>A page from the app's files in the system webview, under the app's own UI.</p>
    </div>
    <div class="stage">
      <div class="shell card">
        <aside class="side">
          <div class="brand">Reader</div>
          <div class="item active">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
              <path d="M3 2.5h7l3 3v8H3z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" />
            </svg>
            Field notes
          </div>
          <div class="item">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
              <path d="M4 2.5h8v11l-4-3-4 3z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" />
            </svg>
            Saved
          </div>
          <div class="item">
            <svg width="16" height="16" viewBox="0 0 16 16" fill="none">
              <circle cx="8" cy="8" r="5.5" stroke="currentColor" stroke-width="1.4" />
              <path d="M8 5v3l2 1.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
            </svg>
            History
          </div>
          <div class="note">The page runs in the system webview; this sidebar and the panel over it are Skathi.</div>
        </aside>
        <div class="doc">
          <iframe ref="frame" class="web" src="assets/page.html" />
          <div class="panel" data-demo="panel">
            <span class="label">Accent</span>
            <button
              v-for="a in ACCENTS"
              :key="a.name"
              class="swatch"
              :class="{ on: accent === a.name }"
              :data-demo="`accent-${a.name}`"
              :style="{ background: a.color }"
              @click="pick(a)"
            />
          </div>
          <div class="toast" :class="{ shown: !!toast }">
            <span class="ok">✓</span> The page applied <b>{{ toast }}</b>
          </div>
        </div>
      </div>
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
.shell {
  width: 1120px;
  height: 600px;
  display: flex;
  overflow: hidden;
  background: var(--sidebar);
}
.side {
  width: 220px;
  padding: 20px 14px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  border-right: 1px solid var(--border);
}
.brand {
  padding: 0 10px 14px;
  font-family: var(--font-display);
  font-size: 18px;
  font-weight: 600;
  color: var(--text);
}
.item {
  height: 36px;
  padding: 0 10px;
  display: flex;
  align-items: center;
  gap: 10px;
  border-radius: 9px;
  color: var(--body);
  font-size: 14px;
}
.item.active {
  background: var(--accent-bg);
  color: var(--pink);
}
.note {
  margin-top: auto;
  padding: 12px;
  border-radius: 12px;
  background: var(--surface);
  color: var(--muted);
  font-size: 12px;
  line-height: 1.5;
}
.doc {
  position: relative;
  flex: 1;
  padding: 14px;
}
.web {
  width: 100%;
  height: 100%;
  border: 0;
  border-radius: 14px;
}
.panel {
  position: absolute;
  right: 34px;
  top: 34px;
  height: 52px;
  padding: 0 10px 0 16px;
  display: flex;
  align-items: center;
  gap: 10px;
  border-radius: 26px;
  background: #17161ef2;
  border: 1px solid var(--border-strong);
  box-shadow: 0 12px 30px #0008;
}
.label {
  margin-right: 4px;
  font-size: 13px;
  color: var(--body);
}
.swatch {
  width: 32px;
  height: 32px;
  border-radius: 16px;
  border: 3px solid #17161e;
  box-shadow: 0 0 0 2px transparent;
  transition:
    box-shadow 160ms ease,
    transform 120ms ease;
}
.swatch.on {
  box-shadow: 0 0 0 2px #ffffff;
}
.swatch:active {
  transform: scale(0.92);
}
.toast {
  position: absolute;
  left: 50%;
  bottom: 34px;
  height: 44px;
  padding: 0 18px;
  display: flex;
  align-items: center;
  gap: 8px;
  transform: translate(-50%, 16px);
  opacity: 0;
  border-radius: 22px;
  background: #17161ef2;
  border: 1px solid var(--border-strong);
  color: var(--text);
  font-size: 14px;
  box-shadow: 0 12px 30px #0008;
  transition:
    opacity 220ms ease,
    transform 260ms cubic-bezier(0.16, 1, 0.3, 1);
}
.toast.shown {
  opacity: 1;
  transform: translate(-50%, 0);
}
.toast b {
  color: var(--pink);
}
.ok {
  width: 20px;
  height: 20px;
  border-radius: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #2b3a33;
  color: #9fe0bd;
  font-size: 12px;
}
</style>
