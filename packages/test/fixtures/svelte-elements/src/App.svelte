<script lang="ts">
  let name = $state("first");
  let surface: HTMLElement | undefined = $state();
  let sticker: HTMLElement | undefined = $state();
  let animation: HTMLElement | undefined = $state();
  const events: string[] = [];

  (globalThis as unknown as { __SK_TEST_SVELTE_ELEMENTS__: unknown }).__SK_TEST_SVELTE_ELEMENTS__ = {
    events,
    rename: (next: string) => (name = next),
    attributes: () => ({
      surface: surface?.getAttribute("name"),
      sticker: [sticker?.getAttribute("src"), sticker?.getAttribute("play")],
      lottie: [animation?.getAttribute("src"), animation?.getAttribute("speed")],
    }),
  };
</script>

<div id="root">
  <sk-surface
    bind:this={surface}
    id="surface"
    class="main"
    {name}
    style="position: absolute; left: 20px; top: 20px; width: 120px; height: 80px"
    onsurfaceresize={(e) => events.push(`surfaceresize:${e.width}x${e.height}`)}
  ></sk-surface>
  <sk-sticker
    bind:this={sticker}
    id="sticker"
    src="/src/steps.json"
    play="off"
    style="position: absolute; left: 160px; top: 20px; width: 80px; height: 80px"
    onload={() => events.push("load:sticker")}
    onerror={() => events.push("error:sticker")}
  ></sk-sticker>
  <lottie
    bind:this={animation}
    id="lottie"
    src="/src/steps.json"
    autoplay
    speed="3"
    style="position: absolute; left: 260px; top: 20px; width: 80px; height: 80px"
    oncomplete={() => events.push("complete:lottie")}
  ></lottie>
</div>
