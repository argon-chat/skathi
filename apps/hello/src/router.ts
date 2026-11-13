// Real vue-router, memory history: no browser location/history needed, which
// is exactly our host's situation. Routes are STATIC imports — the runtime is
// classic-scripts-only, so lazy `() => import(...)` route components are not
// available (the builder would flag them anyway).
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

// `satisfies`, not `: RouteRecordRaw[]`. The annotation would widen `name` to
// `RouteRecordName`, and the `routes.find(...)` below wants the literal names.
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

// `VN_SCENARIO=<route name>` pins the app to one page at boot, so a soak or a
// frame-time measurement is ONE process, ONE screen and no input — which is
// what makes a crash reproducible and a number comparable. The host publishes
// it as a plain global; an app that does not look for it never notices.
const scenario = globalThis.__VN_SCENARIO__;
const pinned = routes.find((r) => r.name === scenario)?.path ?? "/";

// The start route is set on the HISTORY, not by a `replace()` afterwards.
//
// The comment that used to live here said vue-router "only auto-runs the
// initial navigation behind its isBrowser heuristic, so the app kicks it
// explicitly". That was true when it was written and is not true now: vue-router's
// `isBrowser` is `typeof document !== "undefined"`, and this runtime grew a real
// `document`. So `app.use(router)` DOES run its own initial navigation — to
// `routerHistory.location`, i.e. "/" — and it runs it AFTER this module has
// evaluated, quietly undoing any `router.replace()` here. (VN_SCENARIO=media
// logged "pinned=/media" and then "ready at /".)
//
// Seeding the history is the fix that works with the install instead of racing
// it: `routerHistory.location` IS the pinned path, so the navigation vue-router
// runs by itself goes to the right place.
const history = createMemoryHistory();
if (pinned !== "/") history.replace(pinned);

export const router = createRouter({ history, routes });

router
  .isReady()
  .then(() => console.log("[router] ready at " + router.currentRoute.value.fullPath))
  .catch((e: unknown) => console.log("[router] init error: " + e));
