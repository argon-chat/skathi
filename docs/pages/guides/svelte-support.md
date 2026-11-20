---
title: Svelte support
label: Svelte support
description: Which Svelte 5 features work in a Skathi app, which work with limits and which do not, what to use instead, and the sk check code that reports each one.
section: Guides
order: 9
since: "0.4"
---

A Skathi app uses its own Svelte 5 compiler and client runtime. By default Svelte talks to Skathi's DOM, which represents the native element tree and uses Skathi's CSS engine. Runes, blocks, snippets, bindings, transitions and stores work on this path. Features that depend on an HTML parser or registered custom elements have the limits described below.

`sk check` reports every feature on this page that does not work (an error) or works differently (a warning), with codes in the `SK5xxx` range. `sk build`, `sk publish` and `sk dev` print the same lines while they build. `sk explain SK5003` explains a code with an example and a fix. The [quick start](/quick-start/#coming-from-vue-what-differs-in-svelte) covers how a Svelte app mounts and what differs from a Vue app.

## Choose a renderer (since 0.6)

Existing applications and new templates use the **DOM renderer**. It implements the feature table below, including form bindings, transitions and global events. The optional **native renderer** uses Svelte's experimental renderer API and the same native node operations as Skathi's Vue renderer. Both paths draw into the Skathi window.

To try the native renderer, install Svelte 5.59 or later in major 5 and set this option in `sk.config.json`. Keep the existing `mount(App)` entry. Development, hot reload, production bundles and Windows native AOT builds use the setting.

```json
{ "svelte": { "renderer": "native" } }
```

| Feature                                                                     | DOM                  | Native                                                     |
| --------------------------------------------------------------------------- | -------------------- | ---------------------------------------------------------- |
| Runes, stores, context, blocks, keyed lists and Svelte snippets             | Supported            | Supported                                                  |
| Props, component bindings, attachments, actions and callback events         | Supported            | Supported                                                  |
| Scoped CSS, class and style directives                                      | Supported            | Supported through the app stylesheet                       |
| `bind:` on elements, including `bind:this`, dimensions and form controls    | Supported            | Svelte rejects the directive (SK5015)                      |
| Transitions, intros, outros and `animate:flip`                              | Supported            | Svelte rejects the directive (SK5015)                      |
| `<svelte:window>`, `<svelte:document>`, `<svelte:body>` and `<svelte:head>` | Supported            | Svelte rejects the element (SK5015)                        |
| Legacy `on:` on elements                                                    | Supported            | Use an event attribute such as `onclick` (SK5015)          |
| `<svelte:element>`, `<svelte:boundary>`, mounting into a pop-out            | Supported            | Supported                                                  |
| Raw HTML, HTML-string snippets, hydration and registered custom elements    | See the limits below | Raw snippets cannot render, and the same host limits apply |

The CLI collects scoped CSS in the app stylesheet. Native components that request `<svelte:options css="injected" />` receive `SK5015`.

To keep a DOM-based form inside a native application, add `<svelte:options customRenderer={null} />` to its component. Each crossing creates a `<sk-svelte-host>` layout element. Style that element when its dimensions or its place in a flex/grid layout matter. A snippet passed across the boundary must come from a component compiled with the experimental renderer option. Enable the compiler option in the app's `svelte.config.js` when the editor or `svelte-check` reports `svelte_options_customrenderer_disabled`.

```js
export default { compilerOptions: { experimental: { customRenderer: true } } };
```

`SK5015` points to markup the native compiler rejects. `SK5016` means the installed compiler predates this API. Set `svelte.renderer` to `"dom"` or remove the setting to restore the full DOM mode. Skathi pins its compatibility tests to Svelte 5.59.0 and also runs them against the latest 5.59 patch. When upgrading Svelte, run the application's tests with the chosen renderer. [Svelte custom renderers](https://svelte.dev/docs/svelte/custom-renderers)

## At a glance

**Works** means as in a browser. **Limits** means it works with the difference its section describes, and `sk check` warns where it can see it. **Not supported** means it does not work, and `sk check` reports an error where it can see it. **Not supported by design** means Skathi will not add it, and its section says what to use instead.

| Feature                                                                                       | Status                  | sk check |
| --------------------------------------------------------------------------------------------- | ----------------------- | -------- |
| Runes: `$state`, `$derived`, `$effect`, `$props`, `$bindable`, `$inspect`                     | Works                   |          |
| Snippets and `{@render}`, `{#if}`, `{#each}`, `{#await}`, `{#key}`                            | Works                   |          |
| [`{@html}`](#html-strings)                                                                    | Not supported by design | `SK5003` |
| `{@attach}`, actions (`use:`)                                                                 | Works                   |          |
| [`transition:`, `in:`, `out:`, `svelte/transition`, `svelte/easing`](#transitions-and-motion) | Works                   |          |
| [`svelte/motion`: `Tween`, `Spring`](#transitions-and-motion)                                 | Works                   |          |
| [`animate:`, `flip`](#transitions-and-motion)                                                 | Works                   |          |
| [`bind:value` on text fields, `bind:this`, dimension bindings](#bindings)                     | Works                   |          |
| [`bind:value` on `<select>`](#select)                                                         | Works                   |          |
| [Checkboxes and radio buttons, `bind:checked`, `bind:group`](#checkboxes-and-radio-buttons)   | Works                   |          |
| `<svelte:window>`, `<svelte:document>`, `<svelte:body>`, `<svelte:head>`                      | Works                   |          |
| [`<svelte:element>`, `<svelte:boundary>`](#special-elements)                                  | Works                   |          |
| [Errors in event handlers](#errors-in-event-handlers)                                         | Works                   |          |
| [`<svelte:options customElement>`, `$host()`](#custom-elements)                               | Not supported           | `SK5002` |
| `mount`, `unmount` from `@skathi-native/runtime/svelte`                                       | Works                   |          |
| [`hydrate`](#hydrate)                                                                         | Limits: no hydration    | `SK5009` |
| Stores (`svelte/store`, `$store`)                                                             | Works                   |          |
| [Svelte 4 syntax: `export let`, `$:`, `on:`, slots](#svelte-4-syntax)                         | Works                   |          |
| Context: `setContext`, `getContext`                                                           | Works                   |          |
| `svelte/reactivity`: `SvelteMap`, `SvelteSet`, `MediaQuery` and the rest                      | Works                   |          |
| `class:` and `style:` directives, `class` objects and arrays                                  | Works                   |          |
| Scoped `<style>`, `:global()`                                                                 | Works                   |          |
| [`<style lang="scss">` and other preprocessors](#styles)                                      | Limits: plain CSS       | `SK5011` |

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

Not supported by design: Skathi builds its element tree from components and has no HTML parser, so `{@html}` inserts nothing (`SK5003`, an error). Render the content as markup: interpolate plain text with `{text}`, and for rich content such as Markdown, parse it into tokens (`marked.lexer()`, markdown-it's `parse()`) and render each token with a component or a snippet:

```svelte title="Tokens.svelte"
<script lang="ts">
  import type { Token } from "marked";
  import Self from "./Tokens.svelte";

  let { tokens }: { tokens: Token[] } = $props();
</script>

{#each tokens as t}
  {#if t.type === "heading"}<h3><Self tokens={t.tokens ?? []} /></h3>
  {:else if t.type === "paragraph"}<p><Self tokens={t.tokens ?? []} /></p>
  {:else if t.type === "strong"}<strong><Self tokens={t.tokens ?? []} /></strong>
  {:else if t.type === "em"}<em><Self tokens={t.tokens ?? []} /></em>
  {:else if t.type === "codespan"}<code>{t.text}</code>
  {:else if "tokens" in t && t.tokens}<Self tokens={t.tokens} />
  {:else}{t.raw}{/if}
{/each}
```

Render it with `<Tokens tokens={marked.lexer(source)} />`.

## Transitions and motion

Svelte runs `transition:`, `in:` and `out:` through `element.animate()`, which Skathi implements, so the built-in transitions such as `fade`, `fly`, `slide` and `scale`, your own transition functions and the `svelte/easing` functions work, and an outro keeps the element until it ends. `Tween` and `Spring` from `svelte/motion` work.

`animate:` works too: with `flip` from `svelte/animate`, the items that stay in a keyed `{#each}` glide to their new place. `flip` measures each item with `getBoundingClientRect()` before and after the update, and the read after the update lays out the window at once: one extra layout per reorder.

## Bindings

`bind:value` on text inputs and `<textarea>`, `bind:this`, `bind:clientWidth` and the other dimension bindings, `bind:innerWidth` and the other `<svelte:window>` bindings, and `$bindable` props work.

### Select

`bind:value` on a `<select>` works, single and `multiple`: Svelte reads the picked option with `select.querySelector(":checked")`, and `:checked` matches the selected `<option>`.

```svelte
<script lang="ts">
  let size = $state("m");
</script>

<select bind:value={size}>
  <option value="s">Small</option>
  <option value="m">Medium</option>
  <option value="l">Large</option>
</select>
```

### Checkboxes and radio buttons

`<input type="checkbox">` and `<input type="radio">` are native controls. A click, Space or a click on their `<label>` checks them; radio buttons with the same `name` in the same form are a group, and the arrow keys move through it. They fire `click`, then `input` and `change`, and `preventDefault()` on the click undoes the change. `bind:checked`, `bind:indeterminate` and `bind:group` work, and so do `:checked`, `:indeterminate`, `:disabled` and `:focus-visible` in CSS:

```svelte
<script lang="ts">
  let agreed = $state(false);
  let toppings = $state<string[]>([]);
  let size = $state("m");
</script>

<label><input type="checkbox" bind:checked={agreed} /> I agree</label>
<label><input type="checkbox" value="olives" bind:group={toppings} /> Olives</label>
<label><input type="checkbox" value="basil" bind:group={toppings} /> Basil</label>
<label><input type="radio" value="s" bind:group={size} /> Small</label>
<label><input type="radio" value="m" bind:group={size} /> Medium</label>
```

`accent-color` colors the default look. `appearance: none` drops it, so the input draws only its own `background`, `border` and `::before`; a custom checkbox built that way, or as a hidden input inside a styled `<label>`, works as in a browser.

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

### Errors in event handlers

As in a browser, `<svelte:boundary>` does not catch an error thrown by an event handler such as `onclick`: it is reported as uncaught. The console shows it with its stack, `window.onerror` and the window's `error` event receive it, and the event's other listeners still run. Catch what a handler can throw inside it, or listen for the window's `error` event to report it.

## Custom elements

Not supported; may be revisited around 1.0, with no commitment. Skathi has no custom elements and no shadow DOM, which costs too much for Skathi right now (`SK5002`, an error): `customElements.get()` answers `undefined` and `customElements.define()` throws `NotSupportedError`. A component with `<svelte:options customElement="my-element">` compiles to a `customElements.define()` call, which throws when the module loads, and `$host()` only compiles in such a component. Use the component as a component; pass what the host element would carry as props or through context.

## hydrate

There is no server-rendered markup in a window. `hydrate()` from `svelte` renders the component from scratch, as `mount()` does (`SK5009`). Mount with `mount()` from `@skathi-native/runtime/svelte`: it mounts on the window's document root and unmounts with the app when the dev host reloads it.

## Svelte 4 syntax

A component without runes compiles in Svelte 5's legacy mode, and it works: `export let` props, `$:` statements, `on:event` with `createEventDispatcher`, slots with `let:` and fallbacks, and stores with `$store`. Mixing the two works as Svelte allows it, for example a runes component rendering a legacy one. Svelte warns about the old syntax inside a runes component, so keep a component in one mode.

## Styles

Component styles are scoped, `:global()` works, and so do `class:` and `style:` directives. `<style lang="scss">`, `lang="less"` and `lang="stylus"` are not preprocessed (`SK5011`): Skathi's builders do not run `svelte-preprocess`, so the block reaches Svelte's compiler as plain CSS. Write plain CSS (custom properties and nesting cover most of it), or compile the preprocessor's output to a `.css` file and import that. Tailwind CSS 3 works; see [CSS](/css/).
