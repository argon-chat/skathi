<script setup lang="ts">
import { ref } from "vue";

const props = defineProps<{ markdownUrl: string }>();
const state = ref<"idle" | "loading" | "copied" | "error">("idle");
const menu = ref<HTMLDetailsElement>();

async function copyPage(): Promise<void> {
  state.value = "loading";
  try {
    const response = await fetch(props.markdownUrl);
    if (!response.ok) throw new Error("Markdown could not be loaded.");
    await navigator.clipboard.writeText(await response.text());
    state.value = "copied";
    window.setTimeout(() => {
      state.value = "idle";
    }, 2500);
  } catch {
    state.value = "error";
  }
}

function closeMenu(event: FocusEvent): void {
  if (event.relatedTarget instanceof Node && menu.value?.contains(event.relatedTarget)) return;
  if (menu.value) menu.value.open = false;
}
</script>

<template>
  <div class="page-actions">
    <div class="copy-control">
      <button class="copy-page" :disabled="state === 'loading'" @click="copyPage">
        <svg
          width="15"
          height="15"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          aria-hidden="true"
        >
          <rect x="8" y="8" width="12" height="13" rx="2" />
          <path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3" />
        </svg>
        {{ state === "copied" ? "Copied!" : state === "loading" ? "Copying…" : "Copy page" }}
      </button>
      <details ref="menu" class="copy-menu" @focusout="closeMenu" @keydown.esc="menu && (menu.open = false)">
        <summary aria-label="More page options">
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            aria-hidden="true"
          >
            <path d="m6 9 6 6 6-6" />
          </svg>
        </summary>
        <div class="copy-options">
          <a :href="markdownUrl">View Markdown <span aria-hidden="true">↗</span></a>
          <a :href="markdownUrl" download>Download page</a>
          <a href="/llms.txt">Agent documentation</a>
          <a href="/llms-full.txt">Full documentation</a>
        </div>
      </details>
    </div>
    <span v-if="state === 'error'" role="status" class="copy-error"
      >Couldn’t copy. <a :href="markdownUrl">Open Markdown</a></span
    >
    <span v-else class="sr-only" role="status">{{ state === "copied" ? "Page copied as Markdown." : "" }}</span>
  </div>
</template>
