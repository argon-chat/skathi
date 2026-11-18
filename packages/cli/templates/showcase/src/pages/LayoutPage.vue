<script setup lang="ts">
import { computed, ref, shallowRef } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";

const width = ref(520);
let dragFrom: { x: number; width: number } | null = null;
const grab = (e: PointerEvent) => {
  e.preventDefault();
  (e.target as HTMLElement).setPointerCapture(e.pointerId);
  dragFrom = { x: e.clientX, width: width.value };
};
const drag = (e: PointerEvent) => {
  if (dragFrom) width.value = Math.round(Math.min(720, Math.max(220, dragFrom.width + e.clientX - dragFrom.x)));
};
const drop = () => (dragFrom = null);

const people = {
  A: ["Ada Lovelace", "Alan Turing", "Anita Borg"],
  B: ["Barbara Liskov", "Bjarne Stroustrup"],
  C: ["Charles Babbage", "Claude Shannon", "Corrado Böhm"],
  D: ["Dennis Ritchie", "Donald Knuth", "Dorothy Vaughan"],
  E: ["Edsger Dijkstra", "Evelyn Berezin"],
  F: ["Frances Allen", "Fred Brooks"],
};

const slides = [
  { name: "Fjord", fill: "linear-gradient(135deg, #38bdf8, #6366f1)" },
  { name: "Dune", fill: "linear-gradient(135deg, #fbbf24, #f97316)" },
  { name: "Canopy", fill: "linear-gradient(135deg, #4ade80, #0d9488)" },
  { name: "Ember", fill: "linear-gradient(135deg, #fb7185, #c026d3)" },
  { name: "Glacier", fill: "linear-gradient(135deg, #a5f3fc, #818cf8)" },
  { name: "Night", fill: "linear-gradient(135deg, #6366f1, #1e1b4b)" },
];
const strip = shallowRef<HTMLElement | null>(null);
const current = ref(0);
const stop = ref(false);
const step = () => {
  const first = strip.value?.children[0] as HTMLElement | undefined;
  return first ? first.offsetWidth + 12 : 1;
};
const onScroll = () => (current.value = Math.round((strip.value?.scrollLeft ?? 0) / step()));
const go = (i: number) => strip.value?.scrollTo({ left: i * step(), behavior: "smooth" });

const flows = ["row wrap", "row-reverse wrap", "column wrap"];
const flow = ref(flows[0]!);
const pinned = ref<number | null>(null);

const ratios = ["1", "4 / 3", "16 / 9", "21 / 9"];
const ratio = ref("16 / 9");
const ratioLabel = computed(() => ratio.value.replace(" / ", ":"));
</script>

<template>
  <Page
    title="Layout"
    lead="Flexbox and grid with the newer layout CSS: boxes that answer to their own size, headers that stick and scrollers that snap."
  >
    <Card title="Container queries" api="@container · cqi">
      <div class="frame" :style="{ width: `${width}px` }">
        <div class="slot">
          <article class="profile">
            <div class="avatar">AL</div>
            <div class="who">
              <span class="name">Ada Lovelace</span>
              <span class="role">Analyst · London</span>
              <span class="mode mono" />
            </div>
            <div class="stats">
              <span><b>7</b> notes</span>
              <span><b>1</b> engine</span>
              <span><b>1843</b> published</span>
            </div>
            <button class="btn primary follow">Follow</button>
          </article>
        </div>
        <div class="grip" @pointerdown="grab" @pointermove="drag" @pointerup="drop" @lostpointercapture="drop" />
      </div>
      <div class="row">
        <span class="pill mono">{{ width }}px</span>
        <span class="faint">Drag the handle. The card answers to its own width, not the window's.</span>
      </div>
    </Card>

    <Card title="Sticky headers" api="position: sticky">
      <div class="contacts">
        <section v-for="(names, letter) in people" :key="letter" class="group">
          <div class="letter">{{ letter }}</div>
          <div v-for="person in names" :key="person" class="contact">{{ person }}</div>
        </section>
      </div>
      <p class="faint">Scroll the list. Each letter holds at the top until the next one pushes it out.</p>
    </Card>

    <Card title="Scroll snap" api="scroll-snap-type">
      <div
        ref="strip"
        class="strip"
        :class="{ stop }"
        tabindex="0"
        aria-label="Landscapes"
        @scroll="onScroll"
      >
        <div v-for="slide in slides" :key="slide.name" class="slide" :style="{ background: slide.fill }">
          <span class="slide-name">{{ slide.name }}</span>
        </div>
      </div>
      <div class="row">
        <button class="btn" :disabled="current === 0" @click="go(current - 1)">Previous</button>
        <span v-for="(slide, i) in slides" :key="slide.name" class="dot" :class="{ on: i === current }" @click="go(i)" />
        <button class="btn" :disabled="current === slides.length - 1" @click="go(current + 1)">Next</button>
        <label class="toggle">
          <input v-model="stop" type="checkbox" />
          <span class="mono">scroll-snap-stop: always</span>
        </label>
      </div>
      <p class="faint">
        Scroll with the wheel or the scrollbar, or click the strip and use the arrow keys. Every way of scrolling comes
        to rest on a slide.
      </p>
    </Card>

    <Card title="Order and flow" api="order · flex-flow">
      <div class="row">
        <button v-for="f in flows" :key="f" class="btn mono" :class="{ on: flow === f }" @click="flow = f">
          {{ f }}
        </button>
      </div>
      <div class="flow" :style="{ flexFlow: flow }">
        <div
          v-for="n in 6"
          :key="n"
          class="item"
          :class="{ first: pinned === n }"
          @click="pinned = pinned === n ? null : n"
        >
          {{ n }}<span v-if="pinned === n" class="mono"> · order: -1</span>
        </div>
      </div>
      <p class="faint">Click an item to move it to the front with order, without touching the markup.</p>
    </Card>

    <Card title="Aspect ratio" api="aspect-ratio">
      <div class="row">
        <button v-for="r in ratios" :key="r" class="btn mono" :class="{ on: ratio === r }" @click="ratio = r">
          {{ r }}
        </button>
      </div>
      <div class="ratio" :style="{ aspectRatio: ratio }">{{ ratioLabel }}</div>
      <p class="faint">A fixed width and a ratio give the height; a transition between ratios glides.</p>
    </Card>
  </Page>
</template>

<style scoped>
.frame {
  display: flex;
  gap: 6px;
}
.slot {
  flex: 1;
  container: profile / inline-size;
}
.profile {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 16px;
  border-radius: 12px;
  border: 1px solid var(--line);
  background: linear-gradient(135deg, #8b9dff1f, #b48cff14);
  text-align: center;
}
.avatar {
  width: 56px;
  height: 56px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 28px;
  background: linear-gradient(135deg, var(--accent), #b48cff);
  color: #ffffff;
  font-weight: 700;
}
.who {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.name {
  font-size: clamp(15px, 4cqi, 22px);
  font-weight: 700;
}
.role {
  font-size: 12px;
  color: var(--text-muted);
}
.stats {
  display: none;
  gap: 14px;
  font-size: 12px;
  color: var(--text-muted);
}
.stats b {
  color: var(--text);
}
.follow {
  align-self: stretch;
}
.mode {
  margin-top: 4px;
  color: var(--accent);
}
.mode::before {
  content: "stacked";
}
@container profile (width >= 340px) {
  .profile {
    flex-direction: row;
    text-align: left;
  }
  .who {
    flex: 1;
  }
  .follow {
    align-self: center;
  }
  .mode::before {
    content: "in a row";
  }
}
@container profile (width >= 560px) {
  .stats {
    display: flex;
  }
  .mode::before {
    content: "in a row, with stats";
  }
}
.grip {
  flex: 0 0 8px;
  border-radius: 4px;
  background: var(--surface-strong);
  cursor: ew-resize;
  transition: background-color 120ms ease;
}
.grip:hover {
  background: var(--accent);
}
.contacts {
  height: 200px;
  overflow-y: auto;
  border-radius: 10px;
  border: 1px solid var(--line);
  background: #00000030;
}
.letter {
  position: sticky;
  top: 0;
  padding: 6px 14px;
  border-bottom: 1px solid var(--line);
  background: #1b1c28;
  font-size: 12px;
  font-weight: 700;
  color: var(--accent);
}
.contact {
  padding: 9px 14px;
  font-size: 13px;
}
.strip {
  display: flex;
  gap: 12px;
  padding: 4px 24px 12px;
  overflow-x: auto;
  scroll-snap-type: x mandatory;
  scroll-padding: 0 24px;
  border-radius: 12px;
}
.strip:focus {
  box-shadow: inset 0 0 0 1px var(--accent);
}
.slide {
  flex: 0 0 72%;
  height: 150px;
  display: flex;
  align-items: flex-end;
  padding: 16px;
  border-radius: 14px;
  scroll-snap-align: start;
}
.strip.stop .slide {
  scroll-snap-stop: always;
}
.slide-name {
  font-size: 22px;
  font-weight: 700;
  color: #ffffff;
  text-shadow: 0 2px 10px #00000066;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 4px;
  background: var(--surface-strong);
  cursor: pointer;
  transition: background-color 160ms ease;
}
.dot.on {
  background: var(--accent);
}
.toggle {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: 12px;
  color: var(--text-muted);
  accent-color: var(--accent-strong);
}
.flow {
  height: 80px;
  display: flex;
  gap: 8px;
  align-content: flex-start;
}
.item {
  width: 120px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 9px;
  background: var(--surface-hover);
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: background-color 150ms ease;
}
.item:hover {
  background: var(--surface-strong);
}
.item.first {
  order: -1;
  background: var(--accent-strong);
  color: #ffffff;
}
.ratio {
  width: 280px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 12px;
  background: linear-gradient(135deg, #38bdf8, #8b5cf6);
  color: #ffffff;
  font-size: 18px;
  font-weight: 700;
  transition: aspect-ratio 400ms cubic-bezier(0.2, 0, 0, 1);
}
</style>
