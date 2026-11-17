import { createApp } from "vue";
import { createPinia } from "pinia";
import { createMemoryHistory, createRouter } from "vue-router";
import App from "./App.vue";
import { log } from "./log";
import { RouteA, RouteB } from "./parts";

const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: "/", component: RouteA },
    { path: "/b", component: RouteB },
  ],
});

const app = createApp(App);
app.config.errorHandler = (err, _instance, info) => log(`errorHandler:${(err as Error).message}:${info}`);
app.provide("appLevel", "from-app");
app.use(createPinia()).use(router).mount();
