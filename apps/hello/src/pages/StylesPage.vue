<script setup>
// Pure Tailwind utilities — no matching rules in this <style> block at all.
</script>

<template>
  <div class="card">
    <div class="title">Styles — real Tailwind JIT through the native cascade</div>
    <div
      class="flex flex-col gap-2 p-4 rounded-xl bg-white/10 hover:bg-white/25 transition-colors duration-300 border-2 border-emerald-400/70"
    >
      <div class="text-sm font-semibold text-white">Tailwind card</div>
      <div class="text-xs text-white/70">flex flex-col gap-2 p-4 rounded-xl bg-white/10 — hover me</div>
      <div class="text-xs font-mono text-sky-300">hover:bg-white/25 · transition-colors duration-300</div>
    </div>
  </div>
</template>

<style scoped>
.card {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  background: #ffffff14;
  border-radius: 12px;
}
.title {
  font-size: 14px;
  color: #ffffff;
}
</style>
