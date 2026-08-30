import { createApp } from "vue-native";
import App from "./App.vue";

// The cast is `VnMountableApp` (apps/env.d.ts): `mount()` really does take no
// container here, but `createApp`'s RETURN type still says it needs one.
(createApp(App) as unknown as VnMountableApp).mount();
