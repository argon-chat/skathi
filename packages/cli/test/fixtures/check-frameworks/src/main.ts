import { createSSRApp, defineCustomElement, h, type VueElement } from "vue";
import { createMemoryHistory, createRouter, createWebHistory } from "vue-router";
import App from "./App.vue";

export const routers = [createRouter({ history: createWebHistory(), routes: [] }), createMemoryHistory()];
export const element: typeof VueElement = defineCustomElement({ render: () => h("p") });
createSSRApp(App).mount();
