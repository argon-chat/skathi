<script lang="ts">
  import { SkWindow } from "@skathi-native/runtime/svelte";

  const isWindows = navigator.platform === "Win32";
  const total = 10;
  let left = $state(0);
  let timer = 0;

  function start() {
    left = total;
    clearInterval(timer);
    timer = setInterval(() => {
      left -= 1;
      if (left <= 0) clearInterval(timer);
    }, 1000);
  }
  $effect(() => () => clearInterval(timer));

  const ring = $derived(`conic-gradient(#0090ff ${(left / total) * 360}deg, #ffffff26 0deg)`);
</script>

<main class="page">
  {#if left === 0}
    <button onclick={start}>Start a 10-second break</button>
  {:else}
    <p>Break ends in {left} s</p>
  {/if}
  <!-- #region overlay -->
  {#if isWindows}
    <SkWindow
      title="Break timer"
      cover="monitor"
      chrome="hidden"
      backdrop="transparent"
      alwaysOnTop
      ignoreMouseEvents
      focusable={false}
      visible={left > 0}
    >
      <!-- #endregion overlay -->
      <div class="overlay">
        <div class="ring" style:background={ring}>
          <div class="hole">{left}</div>
        </div>
      </div>
    </SkWindow>
  {/if}
</main>

<style>
  .page {
    padding: 24px;
  }
  .overlay {
    display: flex;
    align-items: flex-end;
    justify-content: flex-end;
    height: 100%;
    padding: 48px;
  }
  .ring {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 120px;
    height: 120px;
    border-radius: 60px;
  }
  .hole {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 96px;
    height: 96px;
    border-radius: 48px;
    background: #1b1b1bf0;
    color: #ffffff;
    font-size: 32px;
  }
</style>
