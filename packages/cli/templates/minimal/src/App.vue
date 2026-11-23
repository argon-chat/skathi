<script setup lang="ts">
import { computed, onMounted, ref } from "vue";

const productName = "{{productName}}";
const initial = productName.charAt(0).toUpperCase();

const count = ref(0);
const clicks = computed(() => `${count.value} ${count.value === 1 ? "click" : "clicks"}`);

// A relative URL resolves against the app root in dev and against the packaged assets in a build.
const greeting = ref("");
onMounted(async () => {
  if (typeof fetch !== "function") return;
  const res = await fetch("assets/hello.txt");
  if (res.ok) greeting.value = (await res.text()).trim();
});
</script>

<template>
  <main class="page">
    <section class="card">
      <div class="logo">{{ initial }}</div>
      <h1 class="title">{{ productName }}</h1>
      <p class="lead">Vue on a native GPU renderer. Edit <span class="code">src/App.vue</span> and save.</p>
      <div class="actions">
        <button class="button" @click="count++">Count up</button>
        <button class="button ghost" :disabled="count === 0" @click="count = 0">Reset</button>
      </div>
      <div class="count">{{ clicks }}</div>
    </section>
    <p v-if="greeting" class="hint">{{ greeting }}</p>
  </main>
</template>

<style>
html {
  line-metrics: snapped;
  font-dilation: luminance;
}
.page {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 20px;
  padding: 32px;
  color: #f4f4f5;
  font-family: system-ui, sans-serif;
}
.card {
  width: 380px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  padding: 36px 32px 28px;
  border-radius: 20px;
  background: #18181bb3;
  border: 1px solid #ffffff14;
  box-shadow: 0 24px 60px #00000066;
}
.logo {
  width: 64px;
  height: 64px;
  border-radius: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 30px;
  font-weight: 700;
  color: #ffffff;
  background: linear-gradient(135deg, #42b883, #2f6fd8);
  box-shadow: 0 10px 28px #2f6fd855;
  animation: float 4s ease-in-out infinite;
}
@keyframes float {
  50% {
    transform: translateY(-5px);
  }
}
.title {
  margin: 0;
  font-size: 26px;
  font-weight: 700;
}
.lead {
  margin: 0;
  font-size: 14px;
  line-height: 21px;
  color: #a1a1aa;
  text-align: center;
}
.code {
  font-family: monospace;
  font-size: 13px;
  color: #d4d4d8;
}
.actions {
  display: flex;
  gap: 10px;
  margin-top: 8px;
}
.button {
  padding: 9px 20px;
  border-radius: 10px;
  border-width: 0;
  font-size: 14px;
  font-weight: 600;
  color: #ffffff;
  background: linear-gradient(135deg, #42b883, #2f6fd8);
  box-shadow: 0 6px 18px #2f6fd840;
  cursor: pointer;
  transition:
    transform 120ms ease,
    box-shadow 120ms ease;
}
.button:hover {
  transform: translateY(-1px);
  box-shadow: 0 10px 24px #2f6fd866;
}
.button:active {
  transform: scale(0.97);
}
.button.ghost {
  color: #d4d4d8;
  background: #ffffff10;
  box-shadow: none;
}
.button.ghost:hover {
  background: #ffffff1c;
}
.button:disabled {
  opacity: 0.4;
  cursor: default;
}
.count {
  font-size: 13px;
  color: #71717a;
}
.hint {
  margin: 0;
  font-size: 12px;
  color: #71717a;
}
</style>
