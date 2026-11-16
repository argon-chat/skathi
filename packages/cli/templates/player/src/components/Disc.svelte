<script lang="ts">
  import { BANDS, player } from "../lib/player.svelte";
  import { seedOf } from "../lib/tracks";

  // The spectrum ring is one effect on this box, drawn under the cover. While the music plays it gets the
  // analyser's levels every frame; smoothing and the falling peak caps happen in the effect's history.
  let { accent }: { accent: string } = $props();

  const SIZE = 400;
  const INNER = 118;
  const REACH = 46;

  const levels = new Float32Array(BANDS);
  let shading = $state("");

  function spectrum() {
    const values = Array.from(levels, (v) => v.toFixed(3)).join(", ");
    return `spectrum(${values}, ${INNER}px, ${REACH}px, ${(performance.now() / 1000).toFixed(3)}, ${accent})`;
  }

  $effect(() => {
    if (!player.playing) {
      levels.fill(0);
      shading = spectrum();
      return;
    }
    let frame = requestAnimationFrame(function step() {
      player.levels(levels);
      shading = spectrum();
      frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
  });

  const track = $derived(player.current);
  const art = $derived(
    track && !track.cover
      ? `cover(${seedOf(track.id).toFixed(2)}, ${track.palette.first}, ${track.palette.second}, ${track.palette.accent})`
      : "",
  );
</script>

<div class="disc" style:width="{SIZE}px" style:height="{SIZE}px" style:--shading={shading}>
  <div class="cover" class:playing={player.playing} style:--shading={art}>
    {#if track?.cover}
      {#key track.cover}
        <img class="art" src={track.cover} alt={track.title} />
      {/key}
    {:else}
      <img class="wolf" src="assets/wolf.svg" alt="" />
    {/if}
  </div>
</div>

<style>
  .disc {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }
  .cover {
    width: 220px;
    height: 220px;
    border-radius: 110px;
    overflow: hidden;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #211f2a;
    border: 3px solid #ffffff2e;
    box-shadow:
      0 18px 48px #00000080,
      0 0 0 6px #17161e66;
    transition:
      --shading 900ms ease,
      transform 500ms cubic-bezier(0.3, 1.4, 0.5, 1);
    transform: scale(0.94);
  }
  .cover.playing {
    transform: scale(1);
  }
  .art {
    width: 100%;
    height: 100%;
    object-fit: cover;
    animation: arrive 600ms ease;
  }
  @keyframes arrive {
    from {
      opacity: 0;
      transform: scale(1.08);
    }
  }
  .wolf {
    width: 74px;
    height: 74px;
  }
</style>
