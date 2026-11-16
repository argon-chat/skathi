<script lang="ts">
  import { untrack } from "svelte";
  import { hearts } from "../lib/fx.svelte";
  import { player } from "../lib/player.svelte";
  import Icon from "./Icon.svelte";

  // On a track change the title scatters into glitter (the `glitter` effect at 1), swaps while it is all
  // glitter, and gathers back as the transition on --shading returns it to 0.
  const SCATTER_MS = 320;

  let shown = $state(player.current);
  let scattering = $state(false);

  $effect(() => {
    const next = player.current;
    if (next?.id === untrack(() => shown?.id)) return;
    scattering = true;
    const timer = setTimeout(() => {
      shown = next;
      scattering = false;
    }, SCATTER_MS);
    return () => clearTimeout(timer);
  });

  const liked = $derived(shown ? player.liked.has(shown.id) : false);

  function like(e: MouseEvent) {
    if (!shown) return;
    if (player.toggleLike(shown.id) && e.currentTarget instanceof Element) hearts(e.currentTarget);
  }
</script>

<div class="info">
  <div class="text" class:scattering>
    <h1 class="title">{shown?.title ?? "Nothing to play"}</h1>
    <p class="artist">
      {shown?.artist ?? ""}<span class="dot">·</span><span class="album">{shown?.album ?? ""}</span>
    </p>
  </div>
  <button class="like" class:liked onclick={like} title={liked ? "Unlike" : "Like"} aria-pressed={liked}>
    <Icon name="heart" size={20} filled={liked} />
  </button>
</div>

<style>
  .info {
    width: 100%;
    max-width: 560px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 14px;
  }
  .text {
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    --shading: glitter(0, 3px, 28px);
    transition: --shading 320ms ease;
  }
  .text.scattering {
    --shading: glitter(1, 3px, 28px);
  }
  .title {
    margin: 0;
    font-size: 28px;
    font-weight: 800;
    letter-spacing: -0.3px;
    color: #ffffff;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 480px;
  }
  .artist {
    margin: 0;
    font-size: 14px;
    color: #c8c2d6;
    white-space: nowrap;
  }
  .dot {
    margin: 0 8px;
    color: #7d7690;
  }
  .album {
    color: #9d96b0;
  }
  .like {
    width: 40px;
    height: 40px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 20px;
    border: 1px solid #ffffff1f;
    background: #ffffff0d;
    color: #c8c2d6;
    cursor: pointer;
    transition:
      color 160ms ease,
      background-color 160ms ease,
      transform 160ms ease;
  }
  .like:hover {
    background: #ffffff1a;
    color: #f7a8d0;
  }
  .like:active {
    transform: scale(0.9);
  }
  .like.liked {
    color: #f7a8d0;
    border-color: #f7a8d066;
    background: #f7a8d01f;
  }
</style>
