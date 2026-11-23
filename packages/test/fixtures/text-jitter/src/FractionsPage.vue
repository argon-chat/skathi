<script setup lang="ts">
defineProps<{ tick: number }>();

const names = ["Ada", "Bo", "Cy", "Di", "Ed", "Fa", "Gu"];
</script>

<template>
  <div class="fractions">
    <div class="bar">
      <span id="tick">{{ tick }}</span>
    </div>
    <div class="shares">
      <div v-for="n in names" :key="n" class="tile">
        <span class="face">{{ n[0] }}</span>
        <div class="meta">
          <b>{{ n }}</b
          ><span class="time">10:4{{ n.length }}</span>
        </div>
        <p>Static text</p>
      </div>
    </div>
    <div class="percents">
      <div v-for="n in names" :key="n" class="tile">
        <span class="face">{{ n[0] }}</span>
        <div class="meta">
          <b>{{ n }}</b
          ><span class="time">11:0{{ n.length }}</span>
        </div>
        <p>Static text</p>
      </div>
    </div>
    <div class="rows">
      <div v-for="n in names.slice(0, 3)" :key="n" class="tile row">
        <span class="face">{{ n[0] }}</span>
        <div class="meta">
          <b>{{ n }}</b
          ><span class="time">12:1{{ n.length }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<style>
.fractions {
  position: absolute;
  inset: 0;
  padding: 13px 17px;
  display: flex;
  flex-direction: column;
  gap: 9px;
}
.bar {
  height: 20px;
  font-size: 12px;
  color: var(--muted);
}
.shares,
.percents {
  width: 1001px;
  display: flex;
  gap: 5px;
}
.shares .tile {
  flex: 1;
}
.percents {
  width: 997px;
  gap: 0;
}
.percents .tile {
  width: 14.2857%;
  flex-shrink: 0;
}
.rows {
  width: 301px;
  height: 301px;
  display: flex;
  flex-direction: column;
  gap: 0;
}
.rows .tile {
  flex: 1;
  flex-direction: row;
}
.tile {
  padding: 7px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
  border: 1px solid var(--border);
  background: #211f2a;
}
.tile .face {
  width: 27px;
  height: 27px;
  border-radius: 13.5px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #c9a7f5;
  color: var(--ink);
  font-size: 12px;
  font-weight: 700;
}
.tile p {
  margin: 0;
  color: var(--body);
  font-size: 13px;
}
</style>
