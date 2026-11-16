<script lang="ts">
  import { player } from "../lib/player.svelte";
  import { clock } from "../lib/tracks";
  import Icon from "./Icon.svelte";
  import Slider from "./Slider.svelte";

  let { accent }: { accent: string } = $props();

  let scrub = $state<number | null>(null);
  const shownTime = $derived(scrub ?? player.position);
  const volumeIcon = $derived(
    player.muted || player.volume === 0 ? "mute" : player.volume < 0.5 ? "volumeLow" : "volume",
  );
  const glint = $derived(`glint(0.22, ${player.playing ? 1 : 0}, 26px, ${accent})`);
</script>

<div class="dock">
  <div class="seek">
    <span class="time">{clock(shownTime)}</span>
    <Slider
      label="Seek"
      value={player.position}
      max={player.duration || 1}
      oninput={(v) => (scrub = v)}
      onchange={(v) => {
        scrub = null;
        player.seek(v);
      }}
    />
    <span class="time">{clock(player.current?.duration ?? null)}</span>
  </div>

  <div class="buttons">
    <div class="side">
      <button
        class="tool"
        class:on={player.shuffle}
        title="Shuffle"
        aria-pressed={player.shuffle}
        onclick={() => {
          player.shuffle = !player.shuffle;
          player.save();
        }}
      >
        <Icon name="shuffle" size={18} />
      </button>
      <button
        class="tool"
        class:on={player.repeat !== "off"}
        title="Repeat: {player.repeat}"
        onclick={() => player.cycleRepeat()}
      >
        <Icon name={player.repeat === "one" ? "repeatOne" : "repeat"} size={18} />
      </button>
    </div>

    <div class="transport">
      <button class="skip" title="Previous" onclick={() => player.previous()}>
        <Icon name="prev" size={22} />
      </button>
      <button
        class="play"
        class:playing={player.playing}
        title={player.playing ? "Pause" : "Play"}
        style:--shading={glint}
        onclick={() => player.toggle()}
      >
        {#if player.loading}
          <span class="spinner"></span>
        {:else}
          <Icon name={player.playing ? "pause" : "play"} size={28} />
        {/if}
      </button>
      <button class="skip" title="Next" onclick={() => player.next()}>
        <Icon name="next" size={22} />
      </button>
    </div>

    <div class="side volume">
      <button class="tool" title={player.muted ? "Unmute" : "Mute"} onclick={() => player.toggleMute()}>
        <Icon name={volumeIcon} size={18} />
      </button>
      <Slider
        label="Volume"
        value={player.muted ? 0 : player.volume}
        oninput={(v) => player.setVolume(v)}
        onchange={(v) => player.setVolume(v)}
      />
    </div>
  </div>
</div>

<style>
  .dock {
    width: 100%;
    max-width: 600px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 16px 22px 18px;
    border-radius: 26px;
    --shading: jelly(16px, 14px, 22px, #211f2a8c);
  }
  .seek {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .time {
    width: 38px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: #b3acc4;
    text-align: center;
  }
  .buttons {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .side {
    width: 150px;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .side.volume {
    justify-content: flex-end;
  }
  .transport {
    display: flex;
    align-items: center;
    gap: 18px;
  }
  .tool,
  .skip {
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: #c8c2d6;
    cursor: pointer;
    transition:
      color 140ms ease,
      background-color 140ms ease,
      transform 140ms ease;
  }
  .tool {
    width: 36px;
    height: 36px;
    border-radius: 18px;
  }
  .tool:hover,
  .skip:hover {
    color: #ffffff;
    background: #ffffff14;
  }
  .tool.on {
    color: var(--accent);
  }
  .skip {
    width: 44px;
    height: 44px;
    border-radius: 22px;
  }
  .skip:active,
  .tool:active {
    transform: scale(0.9);
  }
  .play {
    width: 66px;
    height: 66px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 33px;
    border: 2px solid #ffffff59;
    background: linear-gradient(145deg, #fff2f8, var(--accent) 55%, #c9a7f5);
    color: #2a1838;
    cursor: pointer;
    box-shadow: 0 10px 28px #00000066;
    transition:
      --shading 600ms ease,
      transform 160ms cubic-bezier(0.3, 1.6, 0.5, 1);
  }
  .play:hover {
    transform: scale(1.06);
  }
  .play:active {
    transform: scale(0.94);
  }
  .play.playing {
    padding-left: 0;
  }
  .spinner {
    width: 22px;
    height: 22px;
    border-radius: 11px;
    border: 3px solid #2a183833;
    border-top-color: #2a1838;
    animation: spin 700ms linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
