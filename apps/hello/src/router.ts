// Routes are STATIC imports — the runtime is classic-scripts-only, so lazy `() => import(...)`
// route components are not available (the builder would flag them anyway).
import { createRouter, createMemoryHistory, type RouteRecordRaw } from "vue-router";

import AudioPage from "./pages/AudioPage.vue";
import FilesPage from "./pages/FilesPage.vue";
import FormsPage from "./pages/FormsPage.vue";
import ListPage from "./pages/ListPage.vue";
import MediaPage from "./pages/MediaPage.vue";
import MotionPage from "./pages/MotionPage.vue";
import PermissionsPage from "./pages/PermissionsPage.vue";
import StoragePage from "./pages/StoragePage.vue";
import StylesPage from "./pages/StylesPage.vue";
import WindowPage from "./pages/WindowPage.vue";
import LabPage from "./App.vue";

// The annotation would widen `name` to `RouteRecordName`, and the `routes.find(...)` below wants
// the literal names.
const routes = [
  { path: "/", name: "forms", component: FormsPage },
  { path: "/list", name: "list", component: ListPage },
  { path: "/motion", name: "motion", component: MotionPage },
  { path: "/media", name: "media", component: MediaPage },
  { path: "/audio", name: "audio", component: AudioPage },
  { path: "/styles", name: "styles", component: StylesPage },
  { path: "/window", name: "window", component: WindowPage },
  { path: "/permissions", name: "permissions", component: PermissionsPage },
  { path: "/storage", name: "storage", component: StoragePage },
  { path: "/lab", name: "lab", component: LabPage },
  { path: "/desktop", name: "desktop", component: FilesPage },
] satisfies RouteRecordRaw[];

// The host publishes it as a plain global; an app that does not look for it never notices.
const scenario = globalThis.__VN_SCENARIO__;
const pinned = routes.find((r) => r.name === scenario)?.path ?? "/";

const history = createMemoryHistory();
if (pinned !== "/") history.replace(pinned);

export const router = createRouter({ history, routes });

router
  .isReady()
  .then(() => console.log("[router] ready at " + router.currentRoute.value.fullPath))
  .catch((e: unknown) => console.log("[router] init error: " + e));
