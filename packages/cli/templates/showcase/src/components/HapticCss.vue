<script setup lang="ts">
import { appHaptics } from "@skathi-native/runtime";
import { ref } from "vue";
import Card from "./Card.vue";

const gentle = ref(false);
const level = ref(3);
const position = ref(50);
const tool = ref("pencil");
let drag: number | undefined;
let lastGuide: number | undefined;

function move(event: PointerEvent) {
  if (event.pointerId !== drag) return;
  const track = event.currentTarget as HTMLElement;
  const box = track.getBoundingClientRect();
  if (!box.width) return;
  const percent = Math.max(0, Math.min(100, 100 * (event.clientX - box.x) / box.width));
  const nearest = Math.round(percent / 25) * 25;
  const guide = Math.abs(percent - nearest) <= 4 ? nearest : undefined;
  position.value = guide ?? percent;
  if (guide !== undefined && guide !== lastGuide) appHaptics.trigger(track, "snap", event.pointerId);
  lastGuide = guide;
}
function begin(event: PointerEvent) {
  if (event.button !== 0 || drag !== undefined) return;
  drag = event.pointerId;
  lastGuide = undefined;
  (event.currentTarget as HTMLElement).setPointerCapture(drag);
  move(event);
}
function end(event: PointerEvent) {
  if (drag === event.pointerId) drag = undefined;
}
</script>

<template>
  <Card title="CSS feedback" api="haptic">
    <div class="stack" :class="{ gentle }">
      <div class="row">
        <button class="btn press">On press</button>
        <button class="btn activate">On activation</button>
        <button class="btn double">Double tap</button>
        <label class="row"><input v-model="gentle" type="checkbox" class="toggle" />Gentle theme</label>
      </div>
      <label class="row">Scale
        <input v-model="level" type="range" min="1" max="5" class="scale" aria-label="CSS scale" />
        <span>{{ level }} / 5</span>
      </label>
      <label class="muted">Drag across the guides</label>
      <div class="guides" aria-label="Haptic snap guides" @pointerdown="begin" @pointermove="move"
        @pointerup="end" @pointercancel="end" @lostpointercapture="end">
        <span v-for="n in [0, 25, 50, 75, 100]" :key="n" class="line" :style="{ left: n + '%' }"></span>
        <span class="handle" :style="{ left: position + '%' }"></span>
      </div>
      <label class="row">Pen texture
        <select v-model="tool" class="input" aria-label="CSS pen texture">
          <option v-for="t in ['pencil', 'brush', 'eraser', 'ink', 'none']" :key="t" :value="t">{{ t }}</option>
        </select>
      </label>
      <div class="stroke" :data-tool="tool">Hold the pen tip here</div>
    </div>
  </Card>
</template>

<style scoped>
@haptic-pattern --double-tap {
  fallback: tick;
  0ms { intensity: .3; sharpness: .6; }
  70ms { intensity: .65; sharpness: .9; }
}
.stack { --press: tick .4; --step: tick .2; --snap: align .5; }
.gentle { --press: tick .15; --step: tick .1; --snap: align .25; }
.press { haptic: press var(--press); }
.activate { haptic: activate success .6; }
.double { haptic: activate pattern(--double-tap); }
.toggle { haptic: change var(--step); }
.scale { haptic: step var(--step), boundary edge .7; haptic-min-interval: 30ms, 0ms; }
.guides {
  haptic: snap var(--snap);
  position: relative;
  height: 36px;
  margin: 0 9px;
  background: var(--surface);
  border-bottom: 1px solid var(--line);
}
.line, .handle { position: absolute; pointer-events: none; }
.line { top: 8px; height: 19px; width: 1px; background: var(--text-faint); }
.handle { top: 9px; height: 18px; width: 18px; margin-left: -9px; border-radius: 9px; background: var(--accent); }
.stroke {
  haptic-target: pen;
  haptic-texture-intensity: .35;
  padding: 12px;
  text-align: center;
  color: var(--text-muted);
  border: 1px dashed var(--line);
  border-radius: 8px;
}
.stroke[data-tool="pencil"] { haptic-texture: pencil; }
.stroke[data-tool="brush"] { haptic-texture: brush; }
.stroke[data-tool="eraser"] { haptic-texture: eraser; }
.stroke[data-tool="ink"] { haptic-texture: ink; }
</style>
