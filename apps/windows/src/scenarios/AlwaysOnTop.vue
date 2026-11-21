<script setup lang="ts">
import { appWindow } from "skathi:app";
import Icon from "../components/Icon.vue";
import Scenario from "../components/Scenario.vue";
import { enterCompact } from "../store";

const togglePin = () => (appWindow.alwaysOnTop = !appWindow.alwaysOnTop);
</script>

<template>
  <Scenario id="windows/always-on-top">
    <div class="card">
      <div class="card-title">A notes window</div>
      <p class="muted">
        The window shrinks to 360 × 440 with setContentBounds() and shows only the note. Pin it, then open the reference
        window: it comes up under the note instead of over it. The expand button brings the gallery back.
      </p>
      <div class="row">
        <button class="btn accent" data-demo="compact" @click="enterCompact">
          <Icon name="pin" />Shrink to a notes window
        </button>
      </div>
    </div>

    <div class="card">
      <div class="card-title">This window</div>
      <div class="switch" :class="{ on: appWindow.alwaysOnTop }" data-demo="pin-main" @click="togglePin">
        <span class="track"><span class="thumb" /></span>
        <span>Keep the gallery above other windows</span>
      </div>
      <p class="muted">
        HWND_TOPMOST, set without activating the window. Other pinned windows still stack among themselves, and the
        value is not saved: read it from your own settings at startup.
      </p>
    </div>
  </Scenario>
</template>
