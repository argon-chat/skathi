---
title: Vue support
label: Vue support
description: Which Vue 3 features work in a Skathi app, which work with limits and which do not, what to use instead, and the sk check code that reports each one.
section: Guides
order: 8
---

A Skathi app runs Vue 3.5's own `@vue/runtime-core` and compiles its components with Vue's own SFC compiler. Only the renderer underneath is Skathi's: instead of creating DOM nodes in a browser, it builds Skathi's native element tree. So everything Vue does above the DOM (reactivity, components, slots, lifecycle, `<script setup>`, Suspense, KeepAlive) behaves as it does in a browser. The differences are where Vue leans on browser machinery Skathi does not have: an HTML parser, custom elements, Vapor's renderer, native checkboxes.

`sk check` reports every feature on this page that does not work (an error) or works differently (a warning), with codes in the `SK5xxx` range. `sk build`, `sk publish` and `sk dev` print the same lines while they build. `sk explain SK5003` explains a code with an example and a fix.

## At a glance

**Works** means as in a browser. **Limits** means it works with the difference its section describes, and `sk check` warns where it can see it. **Not supported** means it does not work, and `sk check` reports an error where it can see it.

| Feature                                                                              | Status                   | sk check |
| ------------------------------------------------------------------------------------ | ------------------------ | -------- |
| Reactivity, Composition and Options API, `nextTick`                                  | Works                    |          |
| `<script setup>`, `defineProps`, `defineEmits`, `defineModel`, `defineExpose`        | Works                    |          |
| [`<Suspense>`, async `setup`, top-level `await`](#suspense-and-async-components)     | Works                    |          |
| [`defineAsyncComponent`, lazy routes](#suspense-and-async-components)                | Works                    |          |
| [`<Teleport>`](#teleport)                                                            | Works                    |          |
| `<KeepAlive>`, `onActivated`, `onDeactivated`                                        | Works                    |          |
| [`<Transition>` with CSS transitions or JavaScript hooks](#transitions)              | Works                    |          |
| [`<Transition>` with a CSS `animation`](#transitions)                                | Limits: ends at once     | `SK5014` |
| [`<TransitionGroup>`](#transitions)                                                  | Limits: no moves         | `SK5007` |
| [`v-model` on text fields, `<textarea>`, `<select>`, components](#forms-and-v-model) | Works                    |          |
| [`v-model.lazy`](#forms-and-v-model)                                                 | Limits: per keystroke    | `SK5010` |
| [Checkboxes and radio buttons](#checkboxes-and-radio-buttons)                        | Not supported            | `SK5005` |
| Custom directives, `v-memo`, `v-once`, `v-show`, slots, `<component :is>`            | Works                    |          |
| [`v-html`, `innerHTML`](#v-html)                                                     | Not supported            | `SK5003` |
| Template refs, `useTemplateRef`, `provide`/`inject`, `useId`                         | Works                    |          |
| [`app.config.errorHandler`, `onErrorCaptured`](#error-handling)                      | Limits: not `@` handlers |          |
| [`<style scoped>`, `:deep`, `:slotted`, `:global`, `v-bind()`](#styles)              | Works                    |          |
| [`<style module>`, `useCssModule()`](#styles)                                        | Not supported            | `SK5004` |
| [`<style lang="scss">` and other preprocessors](#styles)                             | Limits: plain CSS        | `SK5011` |
| Render functions, `h()`                                                              | Works                    |          |
| [JSX and TSX](#jsx-and-tsx)                                                          | Limits: not in `sk dev`  | `SK5012` |
| [Vapor mode](#vapor-mode)                                                            | Not supported            | `SK5001` |
| [`defineCustomElement`](#custom-elements)                                            | Not supported            | `SK5002` |
| [`createSSRApp`, `hydrate`](#ssr-and-hydration)                                      | Limits: no hydration     | `SK5009` |
| [vue-router with `createMemoryHistory`](#vue-router)                                 | Works                    |          |
| [vue-router with web or hash history](#vue-router)                                   | Limits: no back          | `SK5008` |
| Pinia                                                                                | Works                    |          |
| [Vue Devtools](#devtools)                                                            | Not supported            |          |

A dependency that uses any of these gets one `SK5013` warning per package, with the places listed, as dependencies' CSS gets `SK4005`.

## Suspense and async components

`<Suspense>` works as in a browser: it shows `#fallback` while an async `setup` (a `<script setup>` with a top-level `await`) is pending, then swaps in the content, and fires `@pending` and `@resolve`. Vue still marks Suspense experimental and logs that once in development.

```vue title="UserCard.vue"
<script setup lang="ts">
const user = await fetch("https://api.example.com/me").then((r) => r.json());
</script>

<template>
  <p>{{ user.name }}</p>
</template>
```

```vue title="App.vue"
<template>
  <Suspense>
    <UserCard />
    <template #fallback><p>Loading…</p></template>
  </Suspense>
</template>
```

`defineAsyncComponent` works with a plain loader and with its options (`loadingComponent`, `delay` and the rest), and so do lazy route components. Write the `import()` with a string literal: `sk dev` resolves only those.

## Teleport

`<Teleport to="body">` puts its content at the end of the window's document root (`body` and `html` are the same root here). A selector works as in a browser, with the same rule: the target must exist when the Teleport mounts. A target that the same component renders later needs `defer`:

```vue
<template>
  <Teleport to="#toasts" defer><Toast /></Teleport>
  <div id="toasts"></div>
</template>
```

`disabled` keeps the content in place. Teleporting into a pop-out window works through `<SkWindow>`; see [Pop-out windows](/guides/pop-out-windows/).

## Transitions

`<Transition>` applies the `*-enter-from`, `*-enter-active`, `*-enter-to` and leave classes, waits for the CSS transition to end, and calls the JavaScript hooks (`@before-enter`, `@enter`, `@after-enter`, `@leave`, `@after-leave`). A hook that takes `done` decides itself when the phase ends.

It does not wait for a CSS `animation`. When the active classes run keyframes instead of a transition, the element enters at once and is removed at once when it leaves, so the animation is cut short or never shows (`SK5014`, reported when the component's own styles do this). Write the effect as a transition, or play the keyframes from a hook with `element.animate()`:

```vue
<script setup lang="ts">
const pop = [
  { transform: "scale(0.8)", opacity: 0 },
  { transform: "scale(1)", opacity: 1 },
];
const enter = (el: Element, done: () => void) => el.animate(pop, 200).finished.then(done);
const leave = (el: Element, done: () => void) =>
  el.animate(pop, { duration: 200, direction: "reverse" }).finished.then(done);
</script>

<template>
  <Transition :css="false" @enter="enter" @leave="leave">
    <Dialog v-if="open" />
  </Transition>
</template>
```

`<TransitionGroup>` plays enter and leave transitions on list items, but not moves: items that stay in the list jump to their new place, and the `-move` class and `moveClass` are ignored. A FLIP animation measures where an item was and where it lands within one frame, and Skathi lays out the new order only on the next frame. `sk check` reports a `<TransitionGroup>` that has a `move-class` or whose component styles a `-move` class (`SK5007`).

## Forms and v-model

`v-model` works on text inputs, `<textarea>` and `<select>` (single and `multiple`, with `:value` bindings of any type), with the `.number` and `.trim` modifiers, and on components with `defineModel`, including named models and custom modifiers:

```vue title="Child.vue"
<script setup lang="ts">
const [title, modifiers] = defineModel<string, "upper">("title");
</script>
```

`.lazy` syncs on `change`, and text fields in Skathi do not fire `change`. Skathi syncs a `.lazy` model on every keystroke instead, as plain `v-model` does (`SK5010`). To commit a value only when the user is done, keep a draft and copy it on `blur` or Enter:

```vue
<script setup lang="ts">
const name = defineModel<string>({ required: true });
const draft = ref(name.value);
</script>

<template>
  <input v-model="draft" @blur="name = draft" @keydown.enter="name = draft" />
</template>
```

### Checkboxes and radio buttons

`<input type="checkbox">` and `<input type="radio">` render as text fields: Skathi has no native toggle controls. A click does not check them, and `v-model` on them never updates (`SK5005`, an error). Build the control from a button instead, or use a UI library whose checkbox, switch and radio group are made that way (the reka-ui components in the `shadcn` sample are):

```vue title="Checkbox.vue"
<script setup lang="ts">
const checked = defineModel<boolean>({ default: false });
</script>

<template>
  <button type="button" role="checkbox" :aria-checked="checked" class="checkbox" @click="checked = !checked">
    <span v-if="checked">✓</span>
  </button>
</template>
```

## v-html

Skathi's DOM has no HTML parser. `v-html` and a bound `innerHTML` set an attribute instead of building elements, so the element stays empty (`SK5003`, an error). Render the content as elements: for text, interpolate it with `{{ }}`; for rich content such as Markdown, parse it into tokens and render each token with a component or `h()`.

## Error handling

`app.config.errorHandler` and `onErrorCaptured` receive errors thrown during render, in lifecycle hooks, watchers and async `setup`. An error thrown by an event handler bound in a template (`@click="save"`) does not reach them. Catch what a handler can throw inside it:

```ts
async function save() {
  try {
    await store.save();
  } catch (e) {
    reportError(e);
  }
}
```

## Styles

Scoped styles work, with `:deep()`, `:slotted()` and `:global()`, and so does `v-bind()` in `<style>`: the values are custom properties on the component's root element and follow the state.

`<style module>` is not compiled (`SK5004`, an error). Its rules are added as plain global rules and `$style` is undefined, so a template that reads `$style.red` throws while rendering and the component renders nothing; `useCssModule()` answers `{}`. Use `<style scoped>` with plain class names.

`<style lang="scss">`, `lang="less"` and `lang="stylus"` are not preprocessed (`SK5011`): the block is compiled as plain CSS, and the declarations that use variables or mixins are dropped. Write plain CSS (custom properties and nesting cover most of it), or compile the preprocessor's output to a `.css` file and import that. Tailwind CSS 3 works; see [CSS](/css/).

## JSX and TSX

`sk build` and `sk publish` compile `.tsx` and `.jsx` components with Vue's JSX runtime (`/** @jsxImportSource vue */` or `"jsxImportSource": "vue"` in `tsconfig.json`). `sk dev` does not load the app's own `.tsx` and `.jsx` files yet, so importing one fails in development (`SK5012`). Write the component as an SFC, or with `h()` in a `.ts` file.

## Vapor mode

Skathi renders Vue through runtime-core's virtual-DOM renderer and has no Vapor runtime (`SK5001`, an error). With Vue 3.5 the `vapor` attribute of `<script setup vapor>` is ignored and the component is compiled as a regular one. With Vue 3.6 the compiled component imports Vapor's runtime, which Skathi's `vue` does not export, and the build fails. `createVaporApp`, `defineVaporComponent` and `vaporInteropPlugin` fail the build in either version. Drop `vapor` and mount with `createApp()`: the same component runs unchanged.

## Custom elements

Skathi has no `customElements` registry and no shadow DOM (`SK5002`, an error). `defineCustomElement()` returns a class that throws `NotSupportedError` when it is constructed, and `useHost()` and `useShadowRoot()` answer `null`. Use the component directly, and pass what a host element would carry as props or with `provide`/`inject`.

## SSR and hydration

There is no server-rendered markup in a window. `createSSRApp()` behaves as `createApp()`, and `hydrate()` renders from scratch as `render()` does (`SK5009`). Call `createApp(App).mount()`.

## vue-router

A Skathi window has one document and no address bar. `history.pushState()` keeps the state but not the URL, and `history.back()`, `forward()` and `go()` do nothing. With `createWebHistory()` or `createWebHashHistory()`, `router.push()` works, but `router.back()` and `router.go()` do nothing and the app always starts at `/` (`SK5008`). Use `createMemoryHistory()`, which keeps its own stack:

```ts title="router.ts"
import { createMemoryHistory, createRouter } from "vue-router";

export const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: "/", component: () => import("./pages/Home.vue") },
    { path: "/settings", component: () => import("./pages/Settings.vue") },
  ],
});
```

To start on a page other than `/`, create the history first and call `history.replace("/settings")` on it before creating the router.

## Devtools

The Vue Devtools browser extension and the standalone Vue Devtools app cannot attach to a Skathi window. Skathi's own inspector (F12) shows the element tree, matched CSS rules, console, network and a frame profiler, and the [performance advisories](/guides/performance-advisories/) name the Vue components that re-render. See [Devtools](/guides/devtools/).
