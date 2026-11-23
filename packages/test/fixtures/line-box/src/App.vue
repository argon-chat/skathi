<template>
  <div class="root">
    <p id="n-auto" class="small">Hxgy <span id="s-auto">Hxgy</span></p>
    <p id="n-snapped" class="small snapped">Hxgy <span id="s-snapped">Hxgy</span></p>
    <p id="b-split" class="tall snapped">HHHH</p>
    <p id="b-above" class="tall snapped above">HHHH</p>
    <p id="b-auto-above" class="tall above">HHHH</p>
    <p id="mixed" class="mixed snapped above">HH<span class="big">HH</span></p>
    <div id="row" class="row snapped above">
      <span id="r13" class="r13">HH</span>
      <span id="r26" class="r26">HH</span>
    </div>
    <input id="field" class="field snapped above" value="HH" />
  </div>
</template>

<style>
.root {
  position: absolute;
  inset: 0;
  padding: 8px 16px;
  background: #ffffff;
  color: #000000;
  font-family: "Manrope", sans-serif;
}
p {
  margin: 0 0 8px;
}
.snapped {
  line-metrics: snapped;
}
.above {
  line-leading: above;
}
.small {
  font-size: 11px;
}
.tall {
  font-size: 20px;
  line-height: 60px;
}
.mixed {
  font-size: 13px;
  line-height: 2;
}
.big {
  font-size: 26px;
}
.row {
  display: flex;
  align-items: baseline;
  gap: 24px;
  margin-bottom: 8px;
}
.r13 {
  font-size: 13px;
  line-height: 2;
}
.r26 {
  font-size: 26px;
  line-height: 2;
}
.field {
  width: 200px;
  height: 40px;
  padding: 0;
  border: none;
  outline: none;
  background: #ffffff;
  color: #000000;
  font-family: "Manrope", sans-serif;
  font-size: 20px;
  line-height: 40px;
}
.field::selection {
  background: #00ff00;
}
</style>
