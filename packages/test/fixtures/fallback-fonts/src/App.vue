<script setup lang="ts">
const PUA = "";
const UNCOVERED = "\u{10FFFD}";
</script>

<template>
  <div class="root">
    <div id="plain" class="probe">Hxgy</div>
    <div id="mixed" class="probe listed">Hx{{ PUA }}</div>
    <div id="mixed-fixed" class="probe listed fixed">Hx{{ PUA }}</div>
    <div id="mixed-snapped" class="probe listed snapped">Hx{{ PUA }}</div>
    <div id="pua" class="probe listed">{{ PUA }}</div>
    <div id="pua-ref" class="probe tengwar">{{ PUA }}</div>
    <div id="adjusted" class="probe listed from-font">{{ PUA }}</div>
    <div id="adjusted-mixed" class="probe listed from-font">Hx{{ PUA }}</div>
    <div id="adjusted-ref" class="probe tengwar adjusted-size">{{ PUA }}</div>
    <div id="ratio" class="probe ratio">HHHH</div>
    <div id="ratio-ref" class="probe ratio-size">HHHH</div>
    <div id="installed-first" class="probe installed">{{ PUA }}</div>
    <div id="uncovered" class="probe installed">{{ UNCOVERED }}</div>
    <div id="uncovered-ref" class="probe segoe">{{ UNCOVERED }}</div>
  </div>
</template>

<style>
.root {
  position: absolute;
  inset: 0;
  padding: 8px 16px;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 4px;
  background: #ffffff;
  color: #000000;
  font-family: "Manrope";
}
.probe {
  flex: none;
  font-size: 16px;
  white-space: pre;
}
.listed {
  font-family: "Manrope", "Tengwar";
}
.tengwar {
  font-family: "Tengwar";
}
.fixed {
  line-height: 30px;
}
.snapped {
  line-metrics: snapped;
}
.from-font {
  font-size-adjust: from-font;
}
/* 16px at Manrope's x-height (0.54) over Tengwar's (0.4). */
.adjusted-size {
  font-size: 21.6px;
}
.ratio {
  font-size-adjust: 0.5;
}
/* 16px at 0.5 over Manrope's x-height. */
.ratio-size {
  font-size: 14.8148148px;
}
.installed {
  font-family: "Segoe UI", "Tengwar";
}
.segoe {
  font-family: "Segoe UI";
}
</style>
