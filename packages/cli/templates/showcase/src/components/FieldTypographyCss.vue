<script setup lang="ts">
import Card from "./Card.vue";
</script>

<template>
  <Card title="Growing text field" api="field-sizing: content">
    <textarea class="growing-field" rows="4" aria-label="Growing text field" placeholder="Write a note…" />
    <p class="faint">Type a longer note or press Enter. The field grows up to its width and height limits.</p>
  </Card>
  <Card title="Text box trimming" api="text-box: cap alphabetic">
    <div class="trim-comparison">
      <div class="sample">
        <span class="faint">Untrimmed</span>
        <button class="sample-button" type="button">SAVE</button>
      </div>
      <div class="sample">
        <span class="faint">Cap to baseline</span>
        <button class="sample-button trimmed" type="button">SAVE</button>
      </div>
    </div>
  </Card>
</template>

<style scoped>
.growing-field {
  field-sizing: content;
  align-self: start;
  min-width: 140px;
  max-width: 100%;
  max-height: 124px;
  padding: 8px 10px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--surface);
  color: var(--text);
}
.trim-comparison {
  display: flex;
  gap: 14px;
}
.sample {
  display: flex;
  flex-direction: column;
  align-items: start;
  gap: 6px;
}
.sample-button {
  display: block;
  padding: 8px 14px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--surface);
  color: var(--text);
  font-size: 20px;
  line-height: 36px;
}
.trimmed {
  text-box: cap alphabetic;
}
</style>
