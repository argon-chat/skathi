<script setup lang="ts">
import { appLocale } from "skathi:app";
import { computed, ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { productName } from "../product";

// `{name}` is filled from the arguments to t(). `sk.*` keys reword the SDK's own text (its dialogs and prompts).
appLocale.define({
  en: { greeting: "Welcome to {name}", files: "{count} files selected", today: "Today is a good day to ship." },
  de: { greeting: "Willkommen bei {name}", files: "{count} Dateien ausgewählt", today: "Heute ist ein guter Tag." },
  es: { greeting: "Bienvenido a {name}", files: "{count} archivos seleccionados", today: "Hoy es un buen día." },
  ja: { greeting: "{name} へようこそ", files: "{count} 個のファイルを選択", today: "今日はリリース日和です。" },
});

const LANGUAGES = [null, "en", "de", "es", "ja"] as const;
const language = ref(appLocale.language);
const count = ref(3);
const systemLanguages = navigator.languages.join(", ");

function choose(tag: (typeof LANGUAGES)[number]) {
  appLocale.language = tag;
  language.value = tag;
}

// appLocale is not reactive: reading `language` is what makes this recompute after a switch.
const text = computed(() => {
  void language.value;
  return {
    resolved: appLocale.resolved,
    greeting: appLocale.t("greeting", { name: productName }),
    files: appLocale.t("files", { count: count.value }),
    today: appLocale.t("today"),
  };
});
</script>

<template>
  <Page title="Locale" lead="A message catalog per language; t() picks the best match for the user's languages.">
    <Card title="Language" :api="`resolved: ${text.resolved}`">
      <div class="row">
        <button
          v-for="tag in LANGUAGES"
          :key="tag ?? 'auto'"
          class="btn"
          :class="{ on: language === tag }"
          @click="choose(tag)"
        >
          {{ tag ?? "System" }}
        </button>
      </div>
      <p class="faint">System follows navigator.languages: {{ systemLanguages }}</p>
    </Card>

    <Card title="Messages" api="appLocale.t()">
      <div class="greeting">{{ text.greeting }}</div>
      <div class="row">
        <button class="btn" @click="count++">+1</button>
        <span class="muted">{{ text.files }}</span>
      </div>
      <p class="muted">{{ text.today }}</p>
    </Card>
  </Page>
</template>

<style scoped>
.greeting {
  font-size: 22px;
  font-weight: 700;
}
</style>
