---
title: Svelte support
label: Svelte support
description: Which Svelte 5 features work in a Skathi app, which work with limits and which do not, what to use instead, and the sk check code that reports each one.
section: Guides
order: 9
---

A Svelte app on Skathi is compiled by the app's own `svelte/compiler` (Svelte 5) and runs Svelte's own client runtime. Svelte talks to the DOM directly, and in Skathi that DOM is a native element tree with its own CSS engine. So runes, blocks, snippets, bindings, transitions and stores work as they do in a browser. The differences are where Svelte needs browser machinery Skathi does not have: an HTML parser, custom elements, native checkboxes, and `:checked` on `<option>`.

`sk check` reports every feature on this page that does not work (an error) or works differently (a warning), with codes in the `SK5xxx` range. `sk build`, `sk publish` and `sk dev` print the same lines while they build. `sk explain SK5006` explains a code with an example and a fix. The [quick start](/quick-start/#coming-from-vue-what-differs-in-svelte) covers how a Svelte app mounts and what differs from a Vue app.

## At a glance

**Works** means as in a browser. **Limits** means it works with the difference its section describes, and `sk check` warns where it can see it. **Not supported** means it does not work, and `sk check` reports an error where it can see it.

| Feature                                                                                       | Status               | sk check |
| --------------------------------------------------------------------------------------------- | -------------------- | -------- |
| Runes: `$state`, `$derived`, `$effect`, `$props`, `$bindable`, `$inspect`                     | Works                |          |
| Snippets and `{@render}`, `{#if}`, `{#each}`, `{#await}`, `{#key}`                            | Works                |          |
| [`{@html}`](#html-strings)                                                                    | Not supported        | `SK5003` |
| `{@attach}`, actions (`use:`)                                                                 | Works                |          |
| [`transition:`, `in:`, `out:`, `svelte/transition`, `svelte/easing`](#transitions-and-motion) | Works                |          |
| [`svelte/motion`: `Tween`, `Spring`](#transitions-and-motion)                                 | Works                |          |
| [`animate:`](#transitions-and-motion)                                                         | Limits: no moves     | `SK5007` |
| [`bind:value` on text fields, `bind:this`, dimension bindings](#bindings)                     | Works                |          |
| [`bind:value` on `<select>`](#select)                                                         | Not supported        | `SK5006` |
| [Checkboxes and radio buttons, `bind:checked`, `bind:group`](#checkboxes-and-radio-buttons)   | Not supported        | `SK5005` |
| `<svelte:window>`, `<svelte:document>`, `<svelte:body>`, `<svelte:head>`                      | Works                |          |
| `<svelte:element>`, `<svelte:boundary>`                                                       | Works                |          |
| [`<svelte:options customElement>`, `$host()`](#custom-elements)                               | Not supported        | `SK5002` |
| `mount`, `unmount` from `@skathi-native/runtime/svelte`                                       | Works                |          |
| [`hydrate`](#hydrate)                                                                         | Limits: no hydration | `SK5009` |
| Stores (`svelte/store`, `$store`)                                                             | Works                |          |
| [Svelte 4 syntax: `export let`, `$:`, `on:`, slots](#svelte-4-syntax)                         | Works                |          |
| Context: `setContext`, `getContext`                                                           | Works                |          |
| `svelte/reactivity`: `SvelteMap`, `SvelteSet`, `MediaQuery` and the rest                      | Works                |          |
| `class:` and `style:` directives, `class` objects and arrays                                  | Works                |          |
| Scoped `<style>`, `:global()`                                                                 | Works                |          |
| [`<style lang="scss">` and other preprocessors](#styles)                                      | Limits: plain CSS    | `SK5011` |

A dependency that uses any of these gets one `SK5013` warning per package, with the places listed, as dependencies' CSS gets `SK4005`.

## Runes, blocks and snippets

Everything in the runes API works: `$state` (deep and `.raw`), `$derived` and `$derived.by`, `$effect` and `$effect.pre`, `$props` with defaults and rest props, `$bindable`, and `$inspect`, which prints to the app's console in development. `{#await}` shows the pending, then and catch branches, `{#key}` recreates its content when the key changes, and snippets render through `{@render}` and as props:

```svelte title="List.svelte"
<script lang="ts">
  import type { Snippet } from "svelte";

  let { items, row }: { items: string[]; row: Snippet<[string]> } = $props();
</script>

{#each items as item (item)}
  {@render row(item)}
{/each}
```

`tick()`, `onMount`, `onDestroy`, `untrack` and `flushSync` behave as in a browser.

## HTML strings

Skathi's DOM has no HTML parser, so `{@html}` inserts nothing (`SK5003`, an error). Render the content as markup: interpolate plain text with `{text}`, and for rich content such as Markdown, parse it into tokens and render each token with a component or a snippet.

## Transitions and motion

Svelte runs `transition:`, `in:` and `out:` through `element.animate()`, which Skathi implements, so the built-in transitions such as `fade`, `fly`, `slide` and `scale`, your own transition functions and the `svelte/easing` functions work, and an outro keeps the element until it ends. `Tween` and `Spring` from `svelte/motion` work.

`animate:` does not: items that stay in a keyed `{#each}` jump to their new place (`SK5007`). `flip` measures where an item was and where it lands within one frame, and Skathi lays out the new order only on the next frame, so it sees no movement. Enter and leave transitions on the items still play.

## Bindings

`bind:value` on text inputs and `<textarea>`, `bind:this`, `bind:clientWidth` and the other dimension bindings, `bind:innerWidth` and the other `<svelte:window>` bindings, and `$bindable` props work.

### Select

Svelte reads the picked option of a `<select>` with `select.querySelector(":checked")`, and in Skathi `:checked` does not match the selected `<option>`. `bind:value` on a `<select>` therefore always reads the first option (`SK5006`, an error). Set the value and listen for `change` instead:

```svelte
<script lang="ts">
  let size = $state("m");
</script>

<select value={size} onchange={(e) => (size = e.currentTarget.value)}>
  <option value="s">Small</option>
  <option value="m">Medium</option>
  <option value="l">Large</option>
</select>
```

### Checkboxes and radio buttons

`<input type="checkbox">` and `<input type="radio">` render as text fields: Skathi has no native toggle controls. A click does not check them, and `bind:checked` and `bind:group` never update (`SK5005`, an error). Build the control from a button, or use a component library whose checkbox, switch and radio group are made that way (bits-ui's are):

```svelte title="Checkbox.svelte"
<script lang="ts">
  let { checked = $bindable(false) }: { checked?: boolean } = $props();
</script>

<button type="button" role="checkbox" aria-checked={checked} class="checkbox" onclick={() => (checked = !checked)}>
  {#if checked}✓{/if}
</button>
```

## Special elements

`<svelte:window>` and `<svelte:document>` take events and bindings, `<svelte:body>` takes events, and `<svelte:head>` sets `document.title` from a `<title>`. `<svelte:element this={tag}>` switches its tag, and `<svelte:boundary>` catches errors in rendering and effects, with `failed` and `onerror`:

```svelte
<svelte:boundary onerror={(error) => report(error)}>
  <Chart {data} />
  {#snippet failed(error, reset)}
    <p>The chart failed: {(error as Error).message}</p>
    <button onclick={reset}>Try again</button>
  {/snippet}
</svelte:boundary>
```

## Custom elements

Skathi has no `customElements` registry and no shadow DOM (`SK5002`, an error). A component with `<svelte:options customElement="my-element">` compiles to a `customElements.define()` call, which throws a `ReferenceError` when the module loads, and `$host()` only compiles in such a component. Use the component as a component; pass what the host element would carry as props or through context.

## hydrate

There is no server-rendered markup in a window. `hydrate()` from `svelte` renders the component from scratch, as `mount()` does (`SK5009`). Mount with `mount()` from `@skathi-native/runtime/svelte`: it mounts on the window's document root and unmounts with the app when the dev host reloads it.

## Svelte 4 syntax

A component without runes compiles in Svelte 5's legacy mode, and it works: `export let` props, `$:` statements, `on:event` with `createEventDispatcher`, slots with `let:` and fallbacks, and stores with `$store`. Mixing the two works as Svelte allows it, for example a runes component rendering a legacy one. Svelte warns about the old syntax inside a runes component, so keep a component in one mode.

## Styles

Component styles are scoped, `:global()` works, and so do `class:` and `style:` directives. `<style lang="scss">`, `lang="less"` and `lang="stylus"` are not preprocessed (`SK5011`): Skathi's builders do not run `svelte-preprocess`, so the block reaches Svelte's compiler as plain CSS. Write plain CSS (custom properties and nesting cover most of it), or compile the preprocessor's output to a `.css` file and import that. Tailwind CSS 3 works; see [CSS](/css/).
