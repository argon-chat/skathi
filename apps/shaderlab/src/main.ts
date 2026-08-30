import { createApp } from "vue-native";
import App from "./App.vue";

// shaderlab is deliberately the smallest app in the repo: one component, no
// router, no store. What it exercises is the `--shading` effect system, and in
// particular the two APP-AUTHORED effects in ../shaders/.
//
// The cast is `VnMountableApp` (apps/env.d.ts): `mount()` really does take no
// container here, but `createApp`'s RETURN type still says it needs one.
(createApp(App) as unknown as VnMountableApp).mount();
