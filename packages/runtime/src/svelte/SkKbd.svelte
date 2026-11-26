<script lang="ts">
  import type { HTMLAttributes } from "svelte/elements";
  import { kbdPresentation, onKeyLayoutChange, type SkKbdProps } from "@skathi-native/runtime";
  import { createSubscriber } from "svelte/reactivity";

  let {
    keys,
    map,
    platform,
    gamepadLayout,
    mousePrimary,
    variant = "keycaps",
    labels,
    class: className,
    ...attrs
  }: HTMLAttributes<HTMLElement> & SkKbdProps = $props();
  const trackLayout = createSubscriber(onKeyLayoutChange);
  const model = $derived.by(() => {
    trackLayout();
    return kbdPresentation({ keys, map, platform, labels, gamepadLayout, mousePrimary });
  });
</script>

{#if model.groups.length}
  <kbd role="group" aria-label={model.label} {...attrs} class={["sk-kbd", className]} data-scope={map?.scope} data-variant={variant}>
    {#each model.groups as group}
      <span class="sk-kbd-step" aria-hidden="true">
        {#if group.before}<span class="sk-kbd-between">{group.before}</span>{/if}
        {#if variant === "plain"}{group.text}{:else}
          {#each group.keys as key}
            {#if key.joiner}<span class="sk-kbd-join">{key.joiner}</span>{/if}
            <span class="sk-kbd-key" data-device={key.device} data-shape={key.shape}>
              {#if key.glyph}
                <svg
                  class="sk-kbd-glyph"
                  xmlns="http://www.w3.org/2000/svg"
                  viewBox="0 0 24 24"
                  width="20"
                  height="20"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.6"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                >
                  <path d={key.glyph.outline} />
                  {#if key.glyph.fill}<path d={key.glyph.fill} fill="currentColor" stroke="none" />{/if}
                </svg>
                <span>{key.text}</span>
              {:else}{key.text}{/if}
            </span>
          {/each}
        {/if}
      </span>
    {/each}
  </kbd>
{/if}
