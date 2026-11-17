<script setup lang="ts">
import {
  defineAsyncComponent,
  defineComponent,
  h,
  nextTick,
  onErrorCaptured,
  onMounted,
  provide,
  ref,
  useId,
  useTemplateRef,
} from "vue";
import AsyncChild from "./AsyncChild.vue";
import ModelInput from "./ModelInput.vue";
import Styled from "./Styled.vue";
import { log, release } from "./log";
import { InjectChild, KaA, KaB, Thrower, useCounter, vTint } from "./parts";

const LazyImport = defineAsyncComponent(() => import("./Lazy.vue"));
const LazySlow = defineAsyncComponent({
  loader: () => release("lazy").then(() => import("./Lazy.vue")),
  loadingComponent: defineComponent(() => () => h("span", { id: "lazy-loading" }, "loading")),
  delay: 0,
});

const which = ref<"a" | "b">("a");
const show = ref(false);
const items = ref([1, 2, 3]);
let nextItem = 4;

const text = ref("");
const area = ref("");
const picked = ref("b");
const many = ref<string[]>([]);
const num = ref<number | string>(0);
const trimmed = ref("");
const model = ref("start");
const title = ref("old");

const tint = ref("rgb(255, 0, 0)");
const count = ref(0);
const memoKey = ref(0);

const inputEl = useTemplateRef<HTMLInputElement>("input-el");
const listEls = ref<HTMLElement[]>([]);
const refsOut = ref("");

provide("fromParent", "from-parent");
const id1 = useId();
const id2 = useId();
const store = useCounter();

onErrorCaptured((err, _instance, info) => {
  log(`captured:${(err as Error).message}:${info}`);
  return true;
});

onMounted(async () => {
  refsOut.value = `input ${inputEl.value?.tagName ?? "none"}, list ${listEls.value.length}`;
  count.value = 1;
  await nextTick();
  log(`nextTick:${document.querySelector("#count")?.textContent ?? ""}`);
});
</script>

<template>
  <main id="page">
    <section id="suspense">
      <Suspense @pending="log('suspense:pending')" @resolve="log('suspense:resolve')">
        <AsyncChild />
        <template #fallback><p id="sus-fallback">loading</p></template>
      </Suspense>
    </section>

    <section id="teleport">
      <Teleport to="body"><div id="tp-body">in body</div></Teleport>
      <Teleport to="#tp-late" defer><span id="tp-deferred">deferred</span></Teleport>
      <Teleport to="body" disabled><span id="tp-disabled">inline</span></Teleport>
      <div id="tp-late"></div>
    </section>

    <section id="keepalive">
      <button id="ka-switch" @click="which = which === 'a' ? 'b' : 'a'">switch</button>
      <KeepAlive>
        <component :is="which === 'a' ? KaA : KaB" />
      </KeepAlive>
    </section>

    <section id="transition">
      <button id="tr-toggle" @click="show = !show">toggle</button>
      <Transition
        name="fade"
        @before-enter="log('tr:before-enter')"
        @enter="(el: Element) => log(`tr:enter:${el.className}`)"
        @after-enter="(el: Element) => log(`tr:after-enter:${el.className}`)"
        @leave="(el: Element) => log(`tr:leave:${el.className}`)"
        @after-leave="log('tr:after-leave')"
      >
        <p v-if="show" id="tr-el">fading</p>
      </Transition>
      <button id="tg-add" @click="items.push(nextItem++)">add</button>
      <button id="tg-remove" @click="items.shift()">remove</button>
      <TransitionGroup name="list" tag="ul" id="tg">
        <li v-for="n in items" :key="n" class="tg-item">{{ n }}</li>
      </TransitionGroup>
    </section>

    <section id="async">
      <LazyImport label="imported" />
      <LazySlow label="slow" />
    </section>

    <section id="models">
      <input id="m-text" v-model="text" />
      <textarea id="m-area" v-model="area"></textarea>
      <select id="m-select" v-model="picked">
        <option value="a">A</option>
        <option value="b">B</option>
        <option value="c">C</option>
      </select>
      <select id="m-many" v-model="many" multiple>
        <option value="x">X</option>
        <option value="y">Y</option>
      </select>
      <input id="m-num" v-model.number="num" />
      <input id="m-trim" v-model.trim="trimmed" />
      <ModelInput v-model="model" v-model:title.upper="title" />
      <p id="m-out">{{ JSON.stringify({ text, area, picked, many, num, trimmed, model, title }) }}</p>
    </section>

    <section id="directives">
      <p id="tinted" v-tint:arg.mod="tint" @click="tint = 'rgb(0, 0, 255)'">tinted</p>
      <button id="count-inc" @click="count++">inc</button>
      <p id="count">{{ count }}</p>
      <p id="once" v-once>{{ count }}</p>
      <p id="memo" v-memo="[memoKey]">{{ count }}</p>
      <button id="memo-bump" @click="memoKey++">bump</button>
    </section>

    <section id="refs">
      <input ref="input-el" />
      <span v-for="n in 3" :key="n" ref="listEls">{{ n }}</span>
      <p id="refs-out">{{ refsOut }}</p>
      <InjectChild />
      <p id="ids">{{ id1 }} {{ id2 }}</p>
      <button id="store" @click="store.inc()">{{ store.n }} {{ store.doubled }}</button>
    </section>

    <section id="errors">
      <Thrower />
    </section>

    <section id="css">
      <Styled>
        <span id="slotted" class="slotted">slotted</span>
      </Styled>
      <span id="global" class="global-mark">global</span>
    </section>

    <section id="router">
      <RouterView />
      <button id="route-b" @click="$router.push('/b')">b</button>
      <button id="route-back" @click="$router.back()">back</button>
    </section>
  </main>
</template>

<style>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.3s;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
.list-enter-active,
.list-leave-active {
  transition: opacity 0.3s;
}
.list-enter-from,
.list-leave-to {
  opacity: 0;
}
</style>
