<script setup lang="ts">
import { ref } from "vue";

const status = ref("");

function ping() {
  const audio = new AudioContext();
  const tone = audio.createOscillator();
  // #region panner
  const panner = audio.createPanner();
  panner.panningModel = "spatial"; // HRTF wherever no spatial renderer takes it
  panner.positionX.value = 2;
  panner.positionZ.value = -1;
  tone.connect(panner).connect(audio.destination);
  // #endregion panner
  tone.start();
  tone.stop(audio.currentTime + 0.5);

  // #region status
  const spatial = audio.spatialAudio;
  if (spatial?.active) status.value = `${spatial.format}: ${spatial.objectsInUse}/${spatial.maxObjects} objects`;
  else if (spatial?.requested) status.value = `HRTF: ${spatial.reason}`;
  // #endregion status
}
</script>

<template>
  <button @click="ping">Ping from the right</button>
  <p>{{ status }}</p>
</template>
