// Real vue-router, memory history: no browser location/history needed, which
// is exactly our host's situation. Routes are STATIC imports — the runtime is
// classic-scripts-only, so lazy `() => import(...)` route components are not
// available (the builder would flag them anyway).
import { createRouter, createMemoryHistory } from "vue-router";

import FormsPage from "./pages/FormsPage.vue";
import ListPage from "./pages/ListPage.vue";
import MotionPage from "./pages/MotionPage.vue";
import StylesPage from "./pages/StylesPage.vue";
import WindowPage from "./pages/WindowPage.vue";
import LabPage from "./App.vue";

export const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: "/", name: "forms", component: FormsPage },
    { path: "/list", name: "list", component: ListPage },
    { path: "/motion", name: "motion", component: MotionPage },
    { path: "/styles", name: "styles", component: StylesPage },
    { path: "/window", name: "window", component: WindowPage },
    { path: "/lab", name: "lab", component: LabPage },
  ],
});

// vue-router only auto-runs the initial navigation behind its isBrowser
// heuristic; with memory history the app kicks it explicitly (same as SSR).
router.replace("/");
router
  .isReady()
  .then(() => console.log("[router] ready at " + router.currentRoute.value.fullPath))
  .catch((e) => console.log("[router] init error: " + e));
