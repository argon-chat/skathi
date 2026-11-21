<script setup lang="ts">
import type { SkLaunchEvent } from "@skathi-native/runtime";
import { appDevtools, appHost, appLaunch, appShell, appStorage } from "skathi:app";
import { onMounted, onUnmounted, ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { describe } from "../engine";

const status = ref("");
const launches = ref<string[]>([]);
const devtoolsOn = ref(appDevtools.available && appDevtools.isEnabled());

async function run(action: Promise<void>) {
  try {
    await action;
    status.value = "";
  } catch (e) {
    status.value = describe(e);
  }
}

// Fires in the running app when the exe is started again under "singleInstance": true, or opened by a URL it owns.
const onLaunch = (event: Event) => {
  const { source, argv } = event as SkLaunchEvent;
  launches.value = [`${source}: ${argv.join(" ")}`, ...launches.value].slice(0, 4);
};
onMounted(() => appLaunch.addEventListener("launch", onLaunch));
onUnmounted(() => appLaunch.removeEventListener("launch", onLaunch));

const off = appDevtools.onChange((on) => (devtoolsOn.value = on));
onUnmounted(off);
</script>

<template>
  <Page title="System" lead="What a browser cannot do: open things with the system, read how the app was started.">
    <Card title="Shell" api="appShell">
      <div class="row">
        <button class="btn" @click="run(appShell.openExternal('https://github.com/argon-chat/skathi'))">
          Open the Skathi repository
        </button>
        <button v-if="appStorage.folder" class="btn" @click="run(appShell.openPath(appStorage.folder))">
          Open the data folder
        </button>
        <span class="faint">{{ status }}</span>
      </div>
      <p class="faint">openPath refuses programs and scripts unless asked, and openExternal refuses file: URLs.</p>
    </Card>

    <Card title="Launch" api="appLaunch">
      <div class="kv">
        <span class="k">arguments</span><span class="v mono">{{ appLaunch.argv.slice(1).join(" ") || "none" }}</span>
      </div>
      <div class="kv">
        <span class="k">working directory</span><span class="v mono">{{ appLaunch.cwd }}</span>
      </div>
      <div class="kv">
        <span class="k">single instance</span><span class="v">{{ appLaunch.singleInstance ? "yes" : "no" }}</span>
      </div>
      <div v-for="line in launches" :key="line" class="kv">
        <span class="k">launch event</span><span class="v mono">{{ line }}</span>
      </div>
      <p class="faint">
        Set "singleInstance": true and "protocols": ["myapp"] in sk.config.json to receive later launches and myapp://
        links here.
      </p>
    </Card>

    <Card title="Devtools" api="appDevtools">
      <div v-if="appDevtools.available" class="row">
        <span class="pill" :class="{ ok: devtoolsOn }">{{ devtoolsOn ? "on" : "off" }}</span>
        <button class="btn" :disabled="!devtoolsOn" @click="appDevtools.open()">Open (F12)</button>
        <button class="btn" @click="devtoolsOn ? appDevtools.disable() : appDevtools.enable()">
          {{ devtoolsOn ? "Turn off" : "Turn on" }}
        </button>
      </div>
      <p v-else class="muted">This build leaves the devtools out (sk publish does unless you keep them).</p>
      <p class="faint">Element tree, styles, console and network, attached to this window.</p>
    </Card>

    <Card title="Embedding" api="appHost">
      <p class="muted">
        {{
          appHost.available
            ? `Embedded in another program, which offers: ${appHost.functions.join(", ") || "no functions"}.`
            : "Running as its own exe. A program that embeds Skathi as a library gives the app functions to call here."
        }}
      </p>
    </Card>
  </Page>
</template>
