<script setup lang="ts">
import { onMounted, ref } from "vue";
import Icon from "../components/Icon.vue";
import { productName } from "../product";
import { pages } from "../router";
import { useCounter } from "../stores/counter";

const counter = useCounter();
const features = pages.filter((p) => p.blurb);

// A relative URL resolves against the app root in dev and against the packaged assets in a build.
const greeting = ref("");
onMounted(async () => {
  const res = await fetch("assets/hello.txt");
  if (res.ok) greeting.value = (await res.text()).trim();
});
</script>

<template>
  <section class="home">
    <div class="hero">
      <div class="logo">{{ productName.charAt(0) }}</div>
      <h1 class="title">{{ productName }}</h1>
      <p class="lead">
        Vue 3 on a native GPU renderer, with no browser inside. Every page in the sidebar shows one part of Skathi in a
        few lines of code; open <span class="mono">src/pages</span> next to it and change things while
        <span class="mono">sk dev</span> runs.
      </p>
      <div class="row">
        <button class="btn primary" @click="counter.increment()">Count up · {{ counter.count }}</button>
        <button class="btn" :disabled="counter.count === 0" @click="counter.reset()">Reset</button>
      </div>
      <p class="faint">
        The count is a pinia store: the pop-out window, the tray menu and the global hotkey all change the same one.
      </p>
      <p v-if="greeting" class="faint">{{ greeting }}</p>
    </div>

    <div class="grid">
      <RouterLink v-for="page in features" :key="page.path" :to="page.path" class="feature">
        <span class="badge"><Icon :name="page.icon" /></span>
        <span class="name">{{ page.title }}</span>
        <span class="blurb">{{ page.blurb }}</span>
      </RouterLink>
    </div>
  </section>
</template>

<style scoped>
.home {
  display: flex;
  flex-direction: column;
  gap: 32px;
  padding: 40px 32px;
}
.hero {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  text-align: center;
}
.logo {
  width: 72px;
  height: 72px;
  border-radius: 20px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, var(--accent), #b48cff);
  box-shadow: 0 14px 36px #6d7cff55;
  color: #ffffff;
  font-size: 34px;
  font-weight: 700;
  animation: float 5s ease-in-out infinite;
}
@keyframes float {
  50% {
    transform: translateY(-6px);
  }
}
.title {
  margin: 0;
  font-size: 32px;
  font-weight: 700;
}
.lead {
  margin: 0;
  max-width: 560px;
  font-size: 14px;
  line-height: 22px;
  color: var(--text-muted);
}
.grid {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 12px;
}
.feature {
  width: 220px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 16px;
  border-radius: var(--radius);
  border: 1px solid var(--line);
  background: var(--surface);
  cursor: pointer;
  transition:
    background-color 150ms ease,
    border-color 150ms ease,
    transform 150ms ease;
}
.feature:hover {
  background: var(--surface-hover);
  border-color: var(--line-strong);
  transform: translateY(-2px);
}
.badge {
  width: 30px;
  height: 30px;
  border-radius: 9px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--accent-soft);
  color: var(--accent);
}
.name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
}
.blurb {
  font-size: 12px;
  line-height: 17px;
  color: var(--text-muted);
}
</style>
