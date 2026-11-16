<script lang="ts">
  import { fly } from "svelte/transition";
  import { player } from "../lib/player.svelte";
  import { clock, seedOf } from "../lib/tracks";
  import Icon from "./Icon.svelte";
</script>

<aside class="playlist">
  <header class="head">
    <div>
      <h2 class="heading">Up next</h2>
      <p class="count">{player.tracks.length} {player.tracks.length === 1 ? "track" : "tracks"}</p>
    </div>
    <Icon name="sparkle" size={20} />
  </header>

  <ul class="list">
    {#each player.tracks as track, i (track.id)}
      <li class="item" in:fly={{ y: 12, duration: 260 }}>
        <div
          class="row"
          class:current={i === player.index}
          onclick={() => (i === player.index ? player.toggle() : player.select(i))}
          onkeydown={(e) => e.key === "Enter" && player.select(i)}
          role="button"
          tabindex="0"
        >
          <div class="thumb">
            {#if track.cover}
              <img class="thumb-art" src={track.cover} alt="" />
            {:else}
              <div
                class="thumb-art"
                style:--shading="cover({seedOf(track.id).toFixed(2)}, {track.palette.first}, {track.palette.second}, {track
                  .palette.accent})"
              ></div>
            {/if}
            {#if i === player.index}
              <div class="eq" class:paused={!player.playing}>
                <span></span><span></span><span></span>
              </div>
            {/if}
          </div>
          <div class="meta">
            <span class="name">{track.title}</span>
            <span class="by"
              >{#if player.liked.has(track.id)}<span class="heart"><Icon name="heart" size={11} /></span
                >{/if}{track.artist}</span
            >
          </div>
          {#if typeof track.source !== "string"}
            <button
              class="remove"
              title="Remove from the playlist"
              onclick={(e) => {
                e.stopPropagation();
                player.remove(i);
              }}
            >
              <Icon name="close" size={14} />
            </button>
          {/if}
          <span class="length">{clock(track.duration)}</span>
        </div>
      </li>
    {/each}
  </ul>

  <footer class="foot">
    <div class="keys">
      <span><kbd>Space</kbd> play</span>
      <span><kbd>←</kbd><kbd>→</kbd> seek</span>
      <span><kbd>N</kbd> next</span>
    </div>
    <button class="add" onclick={() => player.addFiles()}>
      <Icon name="plus" size={16} />
      Add your music
    </button>
    <p class="formats">mp3 · ogg · flac · wav · m4a</p>
  </footer>
</aside>

<style>
  .playlist {
    width: 340px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    padding: 20px 14px 16px;
    border-radius: 28px;
    --shading: jelly(18px, 16px, 26px, #211f2a99);
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    padding: 0 8px 14px;
    color: var(--accent);
  }
  .heading {
    margin: 0;
    font-size: 18px;
    font-weight: 800;
    color: #ffffff;
  }
  .count {
    margin: 3px 0 0;
    font-size: 12px;
    color: #9d96b0;
  }
  .list {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow-y: auto;
  }
  .item {
    flex-shrink: 0;
    list-style: none;
  }
  .row {
    position: relative;
    height: 60px;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 10px;
    border-radius: 16px;
    cursor: pointer;
    transition: background-color 140ms ease;
  }
  .row:hover {
    background: #ffffff0f;
  }
  .row.current {
    background: #ffffff17;
    box-shadow: inset 0 0 0 1px #ffffff1a;
  }
  .thumb {
    position: relative;
    width: 44px;
    height: 44px;
    flex-shrink: 0;
    border-radius: 12px;
    overflow: hidden;
    background: #2b2836;
  }
  .thumb-art {
    width: 44px;
    height: 44px;
    object-fit: cover;
  }
  .eq {
    position: absolute;
    left: 0;
    top: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    gap: 3px;
    padding-bottom: 12px;
    background: #17161e8c;
  }
  .eq span {
    width: 4px;
    height: 18px;
    border-radius: 2px;
    background: var(--accent);
    animation: bounce 900ms ease-in-out infinite;
  }
  .eq span:nth-child(2) {
    animation-delay: 200ms;
    animation-duration: 700ms;
  }
  .eq span:nth-child(3) {
    animation-delay: 400ms;
    animation-duration: 1100ms;
  }
  .eq.paused span {
    animation: none;
    height: 6px;
  }
  @keyframes bounce {
    0%,
    100% {
      height: 6px;
    }
    50% {
      height: 20px;
    }
  }
  .meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .name {
    font-size: 14px;
    font-weight: 700;
    color: #edeaf3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row.current .name {
    color: var(--accent);
  }
  .by {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: #9d96b0;
    white-space: nowrap;
    overflow: hidden;
  }
  .heart {
    display: flex;
    color: #f7a8d0;
  }
  .length {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: #9d96b0;
  }
  .remove {
    width: 26px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: 13px;
    background: transparent;
    color: #9d96b0;
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .row:hover .remove {
    opacity: 1;
  }
  .remove:hover {
    background: #ffffff1a;
    color: #ffffff;
  }
  .foot {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding-top: 14px;
  }
  .keys {
    display: flex;
    gap: 14px;
    padding-bottom: 6px;
    font-size: 11px;
    color: #8f88a3;
  }
  .keys span {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  kbd {
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 5px;
    border: 1px solid #ffffff24;
    border-bottom-width: 2px;
    background: #ffffff0d;
    font-family: system-ui, sans-serif;
    font-size: 10px;
    color: #d6d0e4;
  }
  .add {
    width: 100%;
    height: 44px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border-radius: 16px;
    border: 1.5px dashed #c9a7f580;
    background: #c9a7f514;
    color: #e6dcfa;
    font-size: 14px;
    font-weight: 700;
    cursor: pointer;
    transition:
      background-color 140ms ease,
      border-color 140ms ease;
  }
  .add:hover {
    background: #c9a7f529;
    border-color: #c9a7f5;
  }
  .formats {
    margin: 0;
    font-size: 11px;
    color: #7d7690;
  }
</style>
