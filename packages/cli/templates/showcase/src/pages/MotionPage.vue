<script setup lang="ts">
import { nextTick, ref, shallowRef } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";

const expanded = ref(false);

const target = shallowRef<HTMLElement | null>(null);
const shake = () =>
  target.value?.animate(
    [
      { transform: "translateX(0px)" },
      { transform: "translateX(-10px)" },
      { transform: "translateX(10px)" },
      { transform: "translateX(0px)" },
    ],
    { duration: 360, easing: "ease-in-out", iterations: 2 },
  );
const spin = () =>
  target.value?.animate([{ transform: "rotate(0deg) scale(1)" }, { transform: "rotate(360deg) scale(1.15)" }], {
    duration: 700,
    easing: "cubic-bezier(0.34, 1.56, 0.64, 1)",
  });

const shown = ref(true);
const words = ["Native", "Fast", "Small", "Vue"];
const word = ref(0);
const nextWord = () => (word.value = (word.value + 1) % words.length);

const tiles = [
  { name: "Dusk", fill: "linear-gradient(135deg, #8b9dff, #b48cff)", text: "Violet light after sundown." },
  { name: "Tide", fill: "linear-gradient(135deg, #38bdf8, #2dd4bf)", text: "Cool water over pale sand." },
  { name: "Ember", fill: "linear-gradient(135deg, #fb923c, #f43f5e)", text: "The last coals of a fire." },
  { name: "Moss", fill: "linear-gradient(135deg, #a3e635, #22c55e)", text: "Green on a north-facing stone." },
];
const faq = [
  {
    q: "Does height: auto animate?",
    a: "Yes. interpolate-size: allow-keywords on an ancestor lets a transition run between a length and auto, measured from the content.",
  },
  {
    q: "What does calc-size() add?",
    a: "Arithmetic on the measured size: calc-size(auto, size * 0.5) is half the content's height, and it animates to and from auto too.",
  },
  {
    q: "Is there script involved?",
    a: "None. The class changes and the renderer measures and interpolates the height on every frame.",
  },
];
const asked = ref<number | null>(0);
const more = ref(false);

let nextId = 7;
const cards = ref([1, 2, 3, 4, 5, 6]);
const shuffle = () => {
  const next = [...cards.value];
  for (let i = next.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [next[i], next[j]] = [next[j]!, next[i]!];
  }
  cards.value = next;
};
const add = () => cards.value.splice(Math.floor(Math.random() * (cards.value.length + 1)), 0, nextId++);
const remove = (n: number) => (cards.value = cards.value.filter((c) => c !== n));

const open = ref<number | null>(null);
const opened = () => tiles[open.value ?? 0]!;
const show = (i: number | null) =>
  document.startViewTransition(async () => {
    open.value = i;
    await nextTick();
  });
</script>

<template>
  <Page
    title="Motion"
    lead="CSS transitions and keyframes run in the renderer; the Web Animations API drives them from script."
  >
    <Card title="Transition" api="transition">
      <div class="track" @click="expanded = !expanded">
        <div class="thumb" :class="{ expanded }">{{ expanded ? "wide" : "narrow" }} · click</div>
      </div>
    </Card>

    <Card title="Keyframes" api="@keyframes">
      <div class="row gap">
        <div class="pulse" />
        <div class="orbit"><div class="moon" /></div>
        <div class="shimmer">Loading…</div>
      </div>
    </Card>

    <Card title="Web Animations" api="element.animate()">
      <div class="row gap">
        <div ref="target" class="box">✦</div>
        <button class="btn" @click="shake">Shake</button>
        <button class="btn" @click="spin">Spin</button>
      </div>
    </Card>

    <Card title="Vue transitions" api="<Transition>">
      <div class="row">
        <button class="btn" @click="shown = !shown">{{ shown ? "Hide" : "Show" }} the panel</button>
        <button class="btn" @click="nextWord">Next word</button>
      </div>
      <Transition name="fade">
        <div v-if="shown" class="panel">Enter and leave classes, as in a browser.</div>
      </Transition>
      <Transition name="swap" mode="out-in">
        <div :key="word" class="word">{{ words[word] }}</div>
      </Transition>
    </Card>

    <Card title="Lists" api="<TransitionGroup>">
      <div class="row">
        <button class="btn" @click="shuffle">Shuffle</button>
        <button class="btn" @click="add">Add</button>
        <span class="faint">Click a card to remove it.</span>
      </div>
      <TransitionGroup name="list" tag="div" class="deck">
        <div v-for="n in cards" :key="n" class="playing" @click="remove(n)">{{ n }}</div>
      </TransitionGroup>
      <p class="faint">Moves play as FLIP transforms; cards enter and leave with @keyframes animations.</p>
    </Card>

    <Card title="Animating to auto" api="interpolate-size · calc-size()">
      <div class="faq">
        <div v-for="(item, i) in faq" :key="item.q" class="qa" :class="{ open: asked === i }">
          <div class="question" @click="asked = asked === i ? null : i">
            <span>{{ item.q }}</span>
            <span class="plus">+</span>
          </div>
          <div class="answer">
            <p class="muted">{{ item.a }}</p>
          </div>
        </div>
      </div>
      <div class="excerpt" :class="{ more }">
        <p class="muted">
          Analytical Engine, 1843. The engine might act upon other things besides number, were objects found whose
          mutual fundamental relations could be expressed by those of the abstract science of operations. Supposing,
          for instance, that the fundamental relations of pitched sounds in the science of harmony and of musical
          composition were susceptible of such expression and adaptations, the engine might compose elaborate and
          scientific pieces of music of any degree of complexity or extent.
        </p>
      </div>
      <div class="row">
        <button class="btn" @click="more = !more">{{ more ? "Show less" : "Read more" }}</button>
        <span class="faint mono">height: calc-size(auto, size * 0.4) ↔ auto</span>
      </div>
    </Card>

    <Card title="View transitions" api="document.startViewTransition()">
      <div v-if="open === null" class="tiles">
        <div
          v-for="(tile, i) in tiles"
          :key="tile.name"
          class="tile"
          :style="{ background: tile.fill, viewTransitionName: `tile-${i}` }"
          @click="show(i)"
        >
          <span class="tile-name" :style="{ viewTransitionName: `tile-name-${i}` }">{{ tile.name }}</span>
        </div>
      </div>
      <div v-else class="detail" @click="show(null)">
        <div class="hero" :style="{ background: opened().fill, viewTransitionName: `tile-${open}` }">
          <span class="hero-name" :style="{ viewTransitionName: `tile-name-${open}` }">{{ opened().name }}</span>
        </div>
        <p class="muted">{{ opened().text }} Click to go back.</p>
      </div>
    </Card>
  </Page>
</template>

<style scoped>
.gap {
  gap: 20px;
}
.track {
  height: 44px;
  padding: 4px;
  border-radius: 12px;
  background: #00000040;
  cursor: pointer;
}
.thumb {
  width: 160px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 9px;
  background: var(--surface-strong);
  font-size: 12px;
  color: var(--text-muted);
  transition:
    width 420ms cubic-bezier(0.34, 1.56, 0.64, 1),
    background-color 420ms ease,
    color 420ms ease;
}
.thumb.expanded {
  width: 420px;
  background: var(--accent-strong);
  color: #ffffff;
}
@keyframes pulse {
  0% {
    transform: scale(0.8);
    opacity: 1;
  }
  100% {
    transform: scale(1.6);
    opacity: 0;
  }
}
.pulse {
  width: 28px;
  height: 28px;
  border-radius: 14px;
  background: var(--accent);
  animation: pulse 1.4s ease-out infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
.orbit {
  width: 48px;
  height: 48px;
  border-radius: 24px;
  border: 1px dashed var(--line-strong);
  animation: spin 3s linear infinite;
}
.moon {
  width: 12px;
  height: 12px;
  margin-left: 18px;
  margin-top: -6px;
  border-radius: 6px;
  background: #b48cff;
}
@keyframes shimmer {
  50% {
    opacity: 0.35;
  }
}
.shimmer {
  padding: 8px 14px;
  border-radius: 8px;
  background: var(--surface-strong);
  font-size: 12px;
  animation: shimmer 1.6s ease-in-out infinite;
}
.box {
  width: 52px;
  height: 52px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 14px;
  background: linear-gradient(135deg, var(--accent), #b48cff);
  color: #ffffff;
  font-size: 22px;
}
.panel {
  padding: 14px;
  border-radius: 10px;
  background: var(--accent-soft);
  font-size: 13px;
}
.fade-enter-active,
.fade-leave-active {
  transition:
    opacity 250ms ease,
    transform 250ms ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
  transform: translateY(-6px);
}
.word {
  font-size: 28px;
  font-weight: 700;
}
.swap-enter-active,
.swap-leave-active {
  transition:
    opacity 180ms ease,
    transform 180ms ease;
}
.swap-enter-from {
  opacity: 0;
  transform: translateY(10px);
}
.swap-leave-to {
  opacity: 0;
  transform: translateY(-10px);
}
.deck {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
}
.playing {
  width: 64px;
  height: 84px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
  border: 1px solid var(--line-strong);
  background: linear-gradient(160deg, #26283a, #1a1b28);
  font-size: 22px;
  font-weight: 700;
  cursor: pointer;
}
.playing:hover {
  border-color: var(--accent);
}
@keyframes deal {
  from {
    opacity: 0;
    transform: translateY(-16px) scale(0.6);
  }
  70% {
    transform: translateY(2px) scale(1.06);
  }
}
@keyframes discard {
  to {
    opacity: 0;
    transform: translateY(16px) rotate(12deg) scale(0.8);
  }
}
.list-move {
  transition: transform 450ms cubic-bezier(0.2, 0, 0, 1);
}
.list-enter-active {
  animation: deal 380ms ease-out;
}
.list-leave-active {
  animation: discard 260ms ease-in forwards;
}
.faq {
  display: flex;
  flex-direction: column;
  interpolate-size: allow-keywords;
  border-radius: 10px;
  border: 1px solid var(--line);
}
.qa + .qa {
  border-top: 1px solid var(--line);
}
.question {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 14px;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
}
.plus {
  font-size: 18px;
  color: var(--text-faint);
  transition: transform 300ms ease;
}
.qa.open .plus {
  transform: rotate(45deg);
}
.answer {
  height: 0;
  overflow: hidden;
  transition: height 300ms cubic-bezier(0.2, 0, 0, 1);
}
.qa.open .answer {
  height: auto;
}
.answer p {
  margin: 0;
  padding: 0 14px 14px;
}
.excerpt {
  height: calc-size(auto, size * 0.4);
  overflow: hidden;
  mask-image: linear-gradient(to bottom, #000 30%, transparent);
  transition: height 400ms cubic-bezier(0.2, 0, 0, 1);
}
.excerpt.more {
  height: auto;
  mask-image: none;
}
.excerpt p {
  margin: 0;
}
.tiles {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 10px;
}
.tile {
  height: 72px;
  display: flex;
  align-items: flex-end;
  padding: 10px;
  border-radius: 12px;
  cursor: pointer;
}
.tile-name {
  font-size: 13px;
  font-weight: 600;
  color: #ffffff;
}
.detail {
  display: flex;
  flex-direction: column;
  gap: 10px;
  cursor: pointer;
}
.hero {
  height: 180px;
  display: flex;
  align-items: flex-end;
  padding: 18px;
  border-radius: 18px;
}
.hero-name {
  font-size: 28px;
  font-weight: 700;
  color: #ffffff;
}
</style>
