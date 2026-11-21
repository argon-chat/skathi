<script setup lang="ts">
import { appPermissions } from "skathi:app";
import { onMounted, onUnmounted, ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { describe } from "../engine";

const rows = ref(appPermissions.list());
const asking = ref("");
const status = ref("");

const read = () => (rows.value = appPermissions.list());
onMounted(() => appPermissions.addEventListener("change", read));
onUnmounted(() => appPermissions.removeEventListener("change", read));

const tone = (state: string) => (state === "granted" ? "ok" : state === "prompt" ? "warn" : "bad");

// Resolves with a fresh read from the OS once the user answers (on Windows, once the Settings toggle moves).
async function request(name: string) {
  asking.value = name;
  try {
    await appPermissions.request(name);
  } catch (e) {
    status.value = describe(e);
  } finally {
    asking.value = "";
    read();
  }
}

async function tryMicrophone() {
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    status.value = stream.getAudioTracks()[0]?.label ?? "a microphone";
    for (const track of stream.getTracks()) track.stop();
  } catch (e) {
    status.value = describe(e);
  }
}
</script>

<template>
  <Page
    title="Permissions"
    lead="What the OS lets this app use, read live. Unlike a browser, an app can ask before it needs the device."
  >
    <Card :title="`On ${appPermissions.platform}`" api="appPermissions">
      <div v-for="row in rows" :key="row.name" class="perm">
        <span class="name">{{ row.name }}</span>
        <span class="pill" :class="tone(row.state)">{{ row.state }}</span>
        <button v-if="row.canRequest" class="btn" :disabled="asking === row.name" @click="request(row.name)">
          {{ asking === row.name ? "Asking…" : "Ask" }}
        </button>
      </div>
    </Card>

    <Card title="Use it" api="getUserMedia()">
      <div class="row">
        <button class="btn" @click="tryMicrophone">Open the microphone</button>
        <span class="faint">{{ status }}</span>
      </div>
      <p class="faint">
        Opens and closes the device at once; the first use asks for consent if nothing was granted yet.
      </p>
    </Card>
  </Page>
</template>

<style scoped>
.perm {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 34px;
}
.name {
  width: 160px;
  font-size: 13px;
}
</style>
