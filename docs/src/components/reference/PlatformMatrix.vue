<script setup lang="ts">
import type { Platform, PlatformSince, Status } from "../../lib/reference";
import StatusMark from "./StatusMark.vue";

defineProps<{ platforms: Record<Platform, Status>; since?: PlatformSince | undefined }>();

const columns: { id: Platform; short: string; label: string }[] = [
  { id: "windows", short: "Win", label: "Windows" },
  { id: "macos", short: "mac", label: "macOS" },
  { id: "linux", short: "Linux", label: "Linux" },
];
const labels: Record<Status, string> = {
  supported: "supported",
  partial: "partial",
  planned: "planned",
  "on-hold": "on hold",
  "not-planned": "not planned",
  deprecated: "deprecated",
};
</script>

<template>
  <span class="platform-matrix">
    <span
      v-for="column in columns"
      :key="column.id"
      class="platform-cell"
      :data-status="platforms[column.id]"
      :title="`${column.label}: ${labels[platforms[column.id]]}${since?.[column.id] ? ` since ${since[column.id]}` : ''}`"
    >
      <StatusMark :status="platforms[column.id]" :size="13" />
      <span>{{ column.short }}</span>
      <span class="sr-only">{{ labels[platforms[column.id]] }}</span>
    </span>
  </span>
</template>
