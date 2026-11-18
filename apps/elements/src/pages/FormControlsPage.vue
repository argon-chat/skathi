<script setup lang="ts">
import { reactive } from "vue";

const DEFAULTS = { autoplay: false, quality: "auto", volume: 40, accent: "#f7a8d0", subtitles: "off" };
const s = reactive({ ...DEFAULTS });
const PRESETS = ["#f7a8d0", "#c9a7f5", "#9fcdf5"];

function reset() {
  Object.assign(s, DEFAULTS);
}
</script>

<template>
  <div class="page">
    <div class="head">
      <span class="tag">checkbox</span>
      <span class="tag">radio</span>
      <span class="tag">range</span>
      <span class="tag">color</span>
      <span class="tag">&lt;select&gt;</span>
      <p>Native form controls, styled with <code>accent-color</code>.</p>
    </div>
    <div class="stage">
      <form class="card form" :style="{ '--acc': s.accent }" @submit.prevent>
        <div class="title">Playback</div>

        <label class="row check" data-demo="autoplay">
          <input v-model="s.autoplay" type="checkbox" class="box" />
          <span>
            <b>Autoplay next episode</b>
            <small>Starts the next one after a short countdown.</small>
          </span>
        </label>

        <div class="row">
          <span class="key">Quality</span>
          <div class="radios">
            <label v-for="q in ['auto', '720p', '1080p']" :key="q" class="radio" :data-demo="`q-${q}`">
              <input v-model="s.quality" type="radio" name="quality" :value="q" />
              {{ q === "auto" ? "Auto" : q }}
            </label>
          </div>
        </div>

        <div class="row">
          <span class="key">Volume</span>
          <input
            v-model.number="s.volume"
            type="range"
            class="range"
            data-demo="volume"
            min="0"
            max="100"
            step="1"
            :style="{ '--pct': `${s.volume}%` }"
          />
          <span class="val">{{ s.volume }}%</span>
        </div>

        <div class="row">
          <span class="key">Accent</span>
          <input v-model="s.accent" type="color" class="color" />
          <div class="presets">
            <button
              v-for="c in PRESETS"
              :key="c"
              type="button"
              class="preset"
              :class="{ on: s.accent === c }"
              :data-demo="`preset-${c.slice(1)}`"
              :style="{ background: c }"
              @click="s.accent = c"
            />
          </div>
          <span class="val mono">{{ s.accent }}</span>
        </div>

        <div class="row">
          <span class="key">Subtitles</span>
          <select v-model="s.subtitles" class="select" data-demo="subtitles">
            <option value="off">Off</option>
            <option value="en">English</option>
            <option value="ja">日本語</option>
            <option value="fr">Français</option>
          </select>
        </div>

        <div class="actions">
          <button type="button" class="btn" data-demo="reset" @click="reset">Reset</button>
          <button type="button" class="btn primary accent">Save</button>
        </div>
      </form>
    </div>
  </div>
</template>

<style scoped>
.head code {
  font-family: var(--mono);
  font-size: 13px;
  color: var(--lilac);
}
.head .tag {
  font-size: 13px;
}
.stage {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
}
.form {
  width: 640px;
  padding: 26px 32px;
  display: flex;
  flex-direction: column;
  gap: 20px;
  accent-color: var(--acc);
}
.title {
  font-family: var(--font-display);
  font-size: 22px;
  font-weight: 600;
}
.row {
  display: flex;
  align-items: center;
  gap: 16px;
  min-height: 36px;
}
.check {
  align-items: flex-start;
  gap: 12px;
}
.check span {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.check b {
  font-weight: 400;
  font-size: 15px;
}
.check small {
  font-size: 13px;
  color: var(--muted);
}
.box {
  width: 20px;
  height: 20px;
  margin-top: 1px;
}
.key {
  width: 92px;
  font-size: 14px;
  color: var(--body);
}
.radios {
  display: flex;
  gap: 22px;
}
.radio {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 15px;
}
.radio input {
  width: 18px;
  height: 18px;
}
.range {
  flex: 1;
  height: 22px;
  appearance: none;
  background: transparent;
}
.range::-webkit-slider-runnable-track {
  height: 6px;
  border-radius: 3px;
  background: linear-gradient(90deg, var(--acc) 0%, var(--acc) var(--pct), #3a3546 var(--pct), #3a3546 100%);
}
.range::-webkit-slider-thumb {
  appearance: none;
  width: 20px;
  height: 20px;
  margin-top: -7px;
  border-radius: 10px;
  background: #ffffff;
  border: 4px solid var(--acc);
}
.val {
  width: 48px;
  text-align: right;
  font-size: 14px;
  color: var(--body);
}
.val.mono {
  width: 78px;
  font-family: var(--mono);
  font-size: 13px;
}
.color {
  width: 56px;
  height: 34px;
  padding: 4px;
  border-radius: 10px;
  border: 1px solid var(--border-strong);
  background: var(--well);
}
.color::-webkit-color-swatch-wrapper {
  padding: 0;
}
.color::-webkit-color-swatch {
  border: 0;
  border-radius: 6px;
}
.presets {
  flex: 1;
  display: flex;
  gap: 10px;
}
.preset {
  width: 28px;
  height: 28px;
  border-radius: 14px;
  border: 3px solid var(--surface);
  box-shadow: 0 0 0 2px transparent;
  transition: box-shadow 160ms ease;
}
.preset.on {
  box-shadow: 0 0 0 2px #ffffff;
}
.select {
  width: 200px;
  height: auto;
  padding: 9px 12px;
  line-height: 18px;
  border-radius: 10px;
  border: 1px solid var(--border-strong);
  background: var(--well);
  color: var(--text);
  font-family: var(--font);
  font-size: 14px;
}
.actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding-top: 4px;
}
.btn.accent {
  background: var(--acc);
  transition: background-color 200ms ease;
}
</style>
