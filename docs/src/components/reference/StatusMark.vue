<script setup lang="ts">
import type { Status } from "../../lib/reference";

withDefaults(defineProps<{ status: Status; size?: number }>(), { size: 16 });
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.8"
    stroke-linecap="round"
    stroke-linejoin="round"
    class="status-mark"
    :data-status="status"
    aria-hidden="true"
  >
    <template v-if="status === 'declared'">
      <path d="m8 6-5 6 5 6m8-12 5 6-5 6" />
    </template>
    <template v-else-if="status === 'supported'">
      <circle cx="12" cy="12" r="9" />
      <path d="m8.2 12.4 2.6 2.6 5-5.4" />
    </template>
    <template v-else-if="status === 'partial'">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 3.6a8.4 8.4 0 0 1 0 16.8Z" fill="currentColor" stroke="none" />
    </template>
    <template v-else-if="status === 'planned'">
      <circle cx="12" cy="12" r="9" stroke-dasharray="2.4 3.2" />
      <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none" />
    </template>
    <template v-else-if="status === 'on-hold'">
      <circle cx="12" cy="12" r="9" />
      <path d="M10 9v6M14 9v6" />
    </template>
    <template v-else-if="status === 'not-planned' || status === 'unsupported'">
      <circle cx="12" cy="12" r="9" />
      <path d="m7.6 16.4 8.8-8.8" />
    </template>
    <template v-else>
      <circle cx="12" cy="12" r="9" />
      <path d="M8 12h8" />
    </template>
  </svg>
</template>
