<script lang="ts">
  import { cubicOut } from "svelte/easing";
  import { Tween } from "svelte/motion";
  import Controls from "./components/Controls.svelte";
  import Disc from "./components/Disc.svelte";
  import Hearts from "./components/Hearts.svelte";
  import Playlist from "./components/Playlist.svelte";
  import Titlebar from "./components/Titlebar.svelte";
  import TrackInfo from "./components/TrackInfo.svelte";
  import { css, hex, look, rgb, type Tint } from "./lib/fx.svelte";
  import { player } from "./lib/player.svelte";

  const palette = $derived(
    player.current?.palette ?? { first: "#f7a8d0", second: "#9fcdf5", accent: "#f7a8d0", heat: 0.3 },
  );

  // The backdrop's colours and mood change through a transition on --shading, blended by the engine. The
  // accent is tweened here instead: the UI reads it as a plain CSS colour, and the spectrum gets it every frame.
  const accent = Tween.of((): Tint => [...rgb(palette.accent), palette.heat], { duration: 900, easing: cubicOut });

  const backdrop = $derived(`${look.scene}(0.05, ${player.playing ? 1 : 0}, ${palette.first}, ${palette.second})`);

  function key(e: KeyboardEvent) {
    if (e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === " " || e.key === "k") player.toggle();
    else if (e.key === "ArrowRight") player.seek(player.position + 5);
    else if (e.key === "ArrowLeft") player.seek(player.position - 5);
    else if (e.key === "ArrowUp") player.setVolume(player.volume + 0.05);
    else if (e.key === "ArrowDown") player.setVolume(player.volume - 0.05);
    else if (e.key === "n") player.next();
    else if (e.key === "p") player.previous();
    else return;
    e.preventDefault();
  }
</script>

<svelte:window onkeydown={key} />

<div class="app" style:--shading={backdrop} style:--accent={css(accent.current)}>
  <Titlebar />
  <main class="stage">
    <section class="now">
      <Disc accent={hex(accent.current, accent.current[3])} />
      <TrackInfo />
      {#if player.error}
        <p class="error">{player.error}</p>
      {/if}
      <Controls accent={hex(accent.current)} />
    </section>
    <Playlist />
  </main>
  <Hearts />
</div>

<style>
  .app {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: #17161e;
    color: #edeaf3;
    font-family: system-ui, sans-serif;
    transition: --shading 1400ms ease;
  }
  .stage {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 24px;
    padding: 0 24px 24px;
  }
  .now {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
  }
  .error {
    margin: 0;
    font-size: 12px;
    color: #f7a8d0;
  }
  @media (max-width: 920px) {
    .stage :global(.playlist) {
      display: none;
    }
  }
</style>
