<script setup lang="ts">
import { ref } from "vue";

const extra = ref("");
const zoomed = ref(false);

(globalThis as { __SK_TEXT_POST__?: unknown }).__SK_TEXT_POST__ = {
  with(classes: string) {
    extra.value = classes;
  },
  zoom(on: boolean) {
    zoomed.value = on;
  },
};

const NAV = ["General", "Appearance", "Notifications", "Privacy & security", "Shortcuts", "About"];
const ROWS = [
  {
    label: "Follow the system theme",
    text: "Switch between light and dark along with Windows.",
    on: true,
  },
  {
    label: "Reduce motion",
    text: "Fade between views instead of sliding them in.",
    on: false,
  },
  {
    label: "Show unread counts",
    text: "Badges on the sidebar count new messages and mentions.",
    on: true,
  },
];
const CARD = {
  title: "Northern Lights",
  by: "Aurora Ensemble · Live at the Observatory",
  meta: ["48 min", "Lossless", "2026"],
};
const NEXT = [
  { title: "Field Notes, Vol. 3", by: "Mira Okafor · Episode 12", time: "36:10" },
  { title: "Slow Tide", by: "Harbour Lights", time: "4:52" },
  { title: "Glass Garden", by: "Noor & the Weather", time: "3:27" },
];
</script>

<template>
  <div :class="['root', extra]">
    <div class="panel">
      <nav class="nav">
        <div class="brand">Preferences</div>
        <div v-for="(item, i) in NAV" :key="item" :class="['item', { active: i === 1 }]">
          <span class="dot" />
          {{ item }}
        </div>
      </nav>
      <section class="body">
        <h1>Appearance</h1>
        <p class="lead">Choose how the app looks on this device. Changes apply to every open window right away.</p>
        <div class="theme">
          <div v-for="(name, i) in ['Light', 'Dark', 'Auto']" :key="name" :class="['swatch', { picked: i === 1 }]">
            <div :class="['preview', name.toLowerCase()]" />
            <span>{{ name }}</span>
          </div>
        </div>
        <div v-for="row in ROWS" :key="row.label" class="row">
          <div class="copy">
            <div class="label">{{ row.label }}</div>
            <div class="text">{{ row.text }}</div>
          </div>
          <div :class="['toggle', { on: row.on }]"><div class="knob" /></div>
        </div>
        <div class="footer">
          <span class="hint">Last saved 2 minutes ago</span>
          <div class="btn">Cancel</div>
          <div class="btn primary">Save changes</div>
        </div>
      </section>
    </div>
    <div class="side">
      <div :class="['media', { zoomed }]">
        <div class="cover" />
        <div class="info">
          <div class="title">{{ CARD.title }}</div>
          <div class="by">{{ CARD.by }}</div>
          <div class="meta">
            <span v-for="m in CARD.meta" :key="m" class="chip">{{ m }}</span>
          </div>
        </div>
        <div class="actions">
          <div class="btn primary">Play</div>
          <div class="btn">Add to queue</div>
        </div>
      </div>
      <div class="queue">
        <div class="queue-head">Up next</div>
        <div v-for="track in NEXT" :key="track.title" class="track">
          <div class="thumb" />
          <div class="copy">
            <div class="label">{{ track.title }}</div>
            <div class="text">{{ track.by }}</div>
          </div>
          <span class="time">{{ track.time }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style>
html,
body {
  background: transparent;
}
.root {
  position: absolute;
  inset: 0;
  padding: 32px;
  display: flex;
  gap: 28px;
  align-items: flex-start;
  font-family: "Segoe UI", sans-serif;
  font-size: 14px;
  color: #edeaf3;
}
.subpixel {
  font-smoothing: subpixel-antialiased;
}
.contrast {
  text-contrast: luminance;
}
.tall {
  line-height: 1.618;
}
.panel {
  display: flex;
  width: 800px;
  border-radius: 18px;
  background: #1d1b24;
  border: 1px solid #ffffff14;
  overflow: hidden;
}
.nav {
  width: 210px;
  padding: 22px 14px;
  background: #18171e;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.brand {
  font-family: Manrope, sans-serif;
  font-weight: 700;
  font-size: 15px;
  padding: 0 10px 14px;
}
.item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border-radius: 8px;
  color: #bcb8c9;
}
.item.active {
  background: #f7a8d01f;
  color: #f7a8d0;
}
.dot {
  width: 14px;
  height: 14px;
  border-radius: 4px;
  background: currentColor;
  opacity: 0.6;
}
.body {
  flex: 1;
  padding: 26px 32px 24px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
h1 {
  margin: 0;
  font-family: Manrope, sans-serif;
  font-weight: 700;
  font-size: 26px;
}
.lead {
  margin: 0;
  color: #bcb8c9;
}
.theme {
  display: flex;
  gap: 14px;
}
.swatch {
  display: flex;
  flex-direction: column;
  gap: 8px;
  align-items: center;
  padding: 10px;
  border-radius: 12px;
  border: 1px solid #ffffff14;
  color: #bcb8c9;
  font-size: 13px;
}
.swatch.picked {
  border-color: #f7a8d0;
  color: #edeaf3;
}
.preview {
  width: 140px;
  height: 48px;
  border-radius: 8px;
}
.preview.light {
  background: linear-gradient(180deg, #f4f2f8 0 30%, #ffffff 30%);
}
.preview.dark {
  background: linear-gradient(180deg, #2a2733 0 30%, #17161e 30%);
}
.preview.auto {
  background: linear-gradient(90deg, #f4f2f8 0 50%, #17161e 50%);
}
.row {
  display: flex;
  align-items: center;
  gap: 24px;
  padding: 14px 0;
  border-top: 1px solid #ffffff12;
}
.copy {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.label {
  font-weight: 600;
}
.text {
  color: #a09aad;
  font-size: 13px;
}
.toggle {
  width: 40px;
  height: 22px;
  border-radius: 11px;
  background: #3a3646;
  padding: 3px;
  display: flex;
}
.toggle.on {
  background: #f7a8d0;
  justify-content: flex-end;
}
.knob {
  width: 16px;
  height: 16px;
  border-radius: 8px;
  background: #ffffff;
}
.footer {
  display: flex;
  align-items: center;
  gap: 10px;
  padding-top: 10px;
}
.hint {
  flex: 1;
  color: #a09aad;
  font-size: 13px;
}
.btn {
  height: 34px;
  padding: 0 16px;
  display: flex;
  align-items: center;
  border-radius: 10px;
  border: 1px solid #ffffff29;
  background: #2a2733;
  color: #edeaf3;
}
.btn.primary {
  border-color: transparent;
  background: #f7a8d0;
  color: #1d1424;
  font-weight: 600;
}
.side {
  width: 372px;
  display: flex;
  flex-direction: column;
  gap: 24px;
}
.media {
  border-radius: 18px;
  background: #1d1b24;
  border: 1px solid #ffffff14;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.media.zoomed {
  transform: scale(1.14);
}
.cover {
  height: 120px;
  background: linear-gradient(135deg, #f7a8d0, #9fcdf5);
  border-radius: 12px;
}
.info {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.title {
  font-family: Manrope, sans-serif;
  font-weight: 700;
  font-size: 18px;
}
.by {
  color: #bcb8c9;
}
.meta {
  display: flex;
  gap: 6px;
  padding-top: 6px;
}
.chip {
  padding: 2px 8px;
  border-radius: 6px;
  background: #ffffff0f;
  color: #a09aad;
  font-size: 12px;
}
.queue {
  border-radius: 18px;
  background: #1d1b24;
  border: 1px solid #ffffff14;
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.time {
  color: #a09aad;
  font-size: 13px;
}
.queue-head {
  font-family: Manrope, sans-serif;
  font-weight: 700;
  font-size: 15px;
}
.track {
  display: flex;
  align-items: center;
  gap: 12px;
}
.thumb {
  width: 40px;
  height: 40px;
  border-radius: 8px;
  background: linear-gradient(135deg, #c9a7f5, #f7a8d0);
}
.actions {
  display: flex;
  gap: 10px;
}
</style>
