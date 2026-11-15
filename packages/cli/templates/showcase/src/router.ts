// Route components are static imports: the runtime loads one bundle, so there is no lazy `() => import()`.
import { createMemoryHistory, createRouter } from "vue-router";
import { nextTick, type Component } from "vue";
import type { IconName } from "./icons";
import AudioPage from "./pages/AudioPage.vue";
import FilesPage from "./pages/FilesPage.vue";
import FormsPage from "./pages/FormsPage.vue";
import GraphicsPage from "./pages/GraphicsPage.vue";
import HomePage from "./pages/HomePage.vue";
import LocalePage from "./pages/LocalePage.vue";
import MediaPage from "./pages/MediaPage.vue";
import MotionPage from "./pages/MotionPage.vue";
import NetworkPage from "./pages/NetworkPage.vue";
import PermissionsPage from "./pages/PermissionsPage.vue";
import ShadersPage from "./pages/ShadersPage.vue";
import StoragePage from "./pages/StoragePage.vue";
import StylesPage from "./pages/StylesPage.vue";
import SystemPage from "./pages/SystemPage.vue";
import TrayPage from "./pages/TrayPage.vue";
import WindowPage from "./pages/WindowPage.vue";

export interface PageEntry {
  path: string;
  title: string;
  icon: IconName;
  section: string;
  blurb: string;
  component: Component;
}

export const pages: PageEntry[] = [
  { path: "/", title: "Home", icon: "home", section: "", blurb: "", component: HomePage },
  {
    path: "/window",
    title: "Window",
    icon: "window",
    section: "Desktop",
    blurb: "Title, size, backdrop, always on top and pop-out windows",
    component: WindowPage,
  },
  {
    path: "/tray",
    title: "Tray & hotkeys",
    icon: "tray",
    section: "Desktop",
    blurb: "A tray icon with a menu, and a shortcut that works from any app",
    component: TrayPage,
  },
  {
    path: "/files",
    title: "Files",
    icon: "files",
    section: "Desktop",
    blurb: "Open and save dialogs, and the clipboard",
    component: FilesPage,
  },
  {
    path: "/permissions",
    title: "Permissions",
    icon: "permissions",
    section: "Desktop",
    blurb: "Microphone and camera access, asked for and watched",
    component: PermissionsPage,
  },
  {
    path: "/system",
    title: "System",
    icon: "system",
    section: "Desktop",
    blurb: "Open links and folders, launch arguments, devtools",
    component: SystemPage,
  },
  {
    path: "/storage",
    title: "Storage",
    icon: "storage",
    section: "Data",
    blurb: "localStorage and IndexedDB, kept on disk between runs",
    component: StoragePage,
  },
  {
    path: "/network",
    title: "Network",
    icon: "network",
    section: "Data",
    blurb: "fetch, WebSocket and an embedded web page",
    component: NetworkPage,
  },
  {
    path: "/media",
    title: "Media",
    icon: "media",
    section: "Media",
    blurb: "Images, video and Lottie stickers from an asset pack",
    component: MediaPage,
  },
  {
    path: "/audio",
    title: "Audio",
    icon: "audio",
    section: "Media",
    blurb: "A Web Audio synth with a live spectrum",
    component: AudioPage,
  },
  {
    path: "/graphics",
    title: "Graphics",
    icon: "graphics",
    section: "Media",
    blurb: "Canvas 2D drawing and a WebGPU shader",
    component: GraphicsPage,
  },
  {
    path: "/shaders",
    title: "Shaders",
    icon: "shaders",
    section: "Media",
    blurb: "Liquid glass, nebula, halftone: the app's own GPU effects in CSS",
    component: ShadersPage,
  },
  {
    path: "/motion",
    title: "Motion",
    icon: "motion",
    section: "Interface",
    blurb: "Transitions, keyframes and the Web Animations API",
    component: MotionPage,
  },
  {
    path: "/styles",
    title: "Styles",
    icon: "styles",
    section: "Interface",
    blurb: "Gradients, shadows, glass and the engine's own shading",
    component: StylesPage,
  },
  {
    path: "/forms",
    title: "Forms",
    icon: "forms",
    section: "Interface",
    blurb: "Native text fields, controls and text selection",
    component: FormsPage,
  },
  {
    path: "/locale",
    title: "Locale",
    icon: "locale",
    section: "Interface",
    blurb: "Message catalogs and the user's languages",
    component: LocalePage,
  },
];

export const router = createRouter({
  history: createMemoryHistory(),
  routes: pages.map(({ path, component }) => ({ path, component })),
});

// Every navigation is a view transition: the route resolves once the old page is captured, and the
// update ends when Vue has rendered the new one.
let rendered: (() => void) | undefined;
router.beforeResolve(
  (to, from) =>
    to.path === from.path ||
    new Promise<void>((proceed) => {
      document.startViewTransition(
        () =>
          new Promise<void>((done) => {
            rendered = done;
            proceed();
          }),
      );
    }),
);
router.afterEach(async () => {
  await nextTick();
  rendered?.();
  rendered = undefined;
});
