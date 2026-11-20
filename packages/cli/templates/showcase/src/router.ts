// Route components are static imports: the runtime loads one bundle, so there is no lazy `() => import()`.
import { createMemoryHistory, createRouter } from "vue-router";
import { nextTick, type Component } from "vue";
import type { IconName } from "./icons";
import AudioPage from "./pages/AudioPage.vue";
import FilesPage from "./pages/FilesPage.vue";
import FormsPage from "./pages/FormsPage.vue";
import GraphicsPage from "./pages/GraphicsPage.vue";
import HapticsPage from "./pages/HapticsPage.vue";
import HomePage from "./pages/HomePage.vue";
import LayoutPage from "./pages/LayoutPage.vue";
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
    blurb: "Animated resizing and a pop-out with live size limits, proportions and opacity",
    component: WindowPage,
  },
  {
    path: "/haptics",
    title: "Haptics",
    icon: "haptics",
    section: "Desktop",
    blurb: "Try device waveforms and choose the feedback for each gesture",
    component: HapticsPage,
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
    blurb: "Images, keyboard-controlled video and Lottie stickers from an asset pack",
    component: MediaPage,
  },
  {
    path: "/audio",
    title: "Audio",
    icon: "audio",
    section: "Media",
    blurb: "A Web Audio synth, a live spectrum and spatial audio",
    component: AudioPage,
  },
  {
    path: "/graphics",
    title: "Graphics",
    icon: "graphics",
    section: "Media",
    blurb: "Pressure-sensitive drawing, haptic guides and a WebGPU shader",
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
    blurb: "Transitions, keyframes, list moves and the Web Animations API",
    component: MotionPage,
  },
  {
    path: "/styles",
    title: "Styles",
    icon: "styles",
    section: "Interface",
    blurb: "Gradients, CSS-driven inline SVG, shadows and engine shading",
    component: StylesPage,
  },
  {
    path: "/layout",
    title: "Layout",
    icon: "layout",
    section: "Interface",
    blurb: "Container queries, sticky headers and tactile scroll snapping",
    component: LayoutPage,
  },
  {
    path: "/forms",
    title: "Forms",
    icon: "forms",
    section: "Interface",
    blurb: "Native text fields, checkboxes, radios and text selection",
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
