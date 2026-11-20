<script setup lang="ts">
import { ref } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";

const video = ref<HTMLVideoElement | null>(null);
const playing = ref(false);
const playbackError = ref("");
const lastKey = ref("");
const imageScale = ref(1);
function zoomImage(event: WheelEvent) {
  if (!event.ctrlKey) return;
  event.preventDefault();
  imageScale.value = Math.min(6, Math.max(0.5, imageScale.value * Math.exp(-event.deltaY / 100)));
}
async function toggleVideo() {
  const player = video.value;
  if (!player) return;
  playbackError.value = "";
  try {
    if (player.paused) await player.play();
    else player.pause();
  } catch (error) {
    playbackError.value = error instanceof Error ? error.message : String(error);
  }
}
function videoKey(event: KeyboardEvent) {
  const location = ["", "left", "right", "numpad"][event.location];
  lastKey.value = [event.code, location, event.shiftKey && "Shift", event.repeat && "held"].filter(Boolean).join(" · ");
  if (event.code !== "Space" || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  event.preventDefault();
  if (!event.repeat) void toggleVideo();
}

const stickerPlaying = ref(true);
const speed = ref(1);

const focal = ref("50% 50%");
const focals = [
  { name: "Left", at: "0% 50%" },
  { name: "Centre", at: "50% 50%" },
  { name: "Right", at: "100% 50%" },
];
</script>

<template>
  <Page
    title="Media"
    lead="Images, video and Lottie animations, loaded from assets/media. sk.config.json puts that folder in its own pack, media.skpk, beside the exe."
  >
    <div class="tiles">
      <Card class="tile" title="Image" api="<img>">
        <div class="image-viewer" @wheel="zoomImage">
          <img
            class="shot"
            src="assets/media/aurora.png"
            alt="Aurora"
            :style="{ transform: 'scale(' + imageScale + ')' }"
          />
        </div>
        <div class="row">
          <button class="btn" :disabled="imageScale === 1" @click="imageScale = 1">Reset zoom</button>
          <span class="muted image-scale">{{ Math.round(imageScale * 100) }}%</span>
        </div>
        <p class="muted">Pinch or use Ctrl+wheel over the image to zoom.</p>
      </Card>

      <Card class="tile" title="Video" api="<video>">
        <video
          ref="video"
          class="shot player"
          src="assets/media/clip.mp4"
          tabindex="0"
          aria-label="Video playback"
          autoplay
          loop
          muted
          @keydown="videoKey"
          @play="playing = true"
          @pause="playing = false"
          @ended="playing = false"
        />
        <div class="row">
          <button class="btn" @click="toggleVideo">{{ playing ? "Pause" : "Play" }}</button>
          <span class="muted">Focus the video and press Space.</span>
        </div>
        <p v-if="lastKey" class="muted mono">{{ lastKey }}</p>
        <p v-if="playbackError" class="muted">{{ playbackError }}</p>
      </Card>

      <Card class="tile" title="Lottie sticker" api="<lottie>">
        <div class="stage">
          <lottie class="sticker" src="assets/media/sticker.tgs" loop :autoplay="stickerPlaying" :speed="speed" />
        </div>
        <div class="row">
          <button class="btn" @click="stickerPlaying = !stickerPlaying">{{ stickerPlaying ? "Pause" : "Play" }}</button>
          <button v-for="s in [0.5, 1, 2]" :key="s" class="btn" :class="{ on: speed === s }" @click="speed = s">
            {{ s }}×
          </button>
        </div>
      </Card>

      <Card class="tile" title="Focal point" api="object-position">
        <div class="crops">
          <img class="crop" src="assets/media/aurora.png" alt="Aurora, cropped" :style="{ objectPosition: focal }" />
          <img class="crop tall" src="assets/media/aurora.png" alt="Aurora, cropped" :style="{ objectPosition: focal }" />
        </div>
        <div class="row">
          <button v-for="f in focals" :key="f.at" class="btn" :class="{ on: focal === f.at }" @click="focal = f.at">
            {{ f.name }}
          </button>
        </div>
      </Card>
    </div>
  </Page>
</template>

<style scoped>
.tiles {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
}
.tile {
  width: 320px;
}
.shot {
  width: 100%;
  height: 160px;
  border-radius: 10px;
  object-fit: cover;
}
.player:focus-visible {
  outline: 2px solid var(--accent);
}
.image-viewer {
  height: 160px;
  overflow: hidden;
  border-radius: 10px;
}
.stage {
  height: 160px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 10px;
  background: radial-gradient(circle, #8b9dff30, #00000000);
}
.crops {
  height: 160px;
  display: flex;
  gap: 10px;
}
.crop {
  flex: 1;
  height: 100%;
  border-radius: 10px;
  object-fit: cover;
  transition: object-position 500ms cubic-bezier(0.2, 0, 0, 1);
}
.tall {
  flex: 0 0 90px;
}
.sticker {
  width: 120px;
  height: 120px;
}
</style>
