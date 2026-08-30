import { createApp } from "vue-native";
import App from "./App.vue";

// shaderlab is deliberately the smallest app in the repo: one component, no
// router, no store. What it exercises is the `--shading` effect system, and in
// particular the two APP-AUTHORED effects in ../shaders/.
createApp(App).mount();
