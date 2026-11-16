---
title: Performance advisories
label: Performance advisories
description: The warnings a development build prints when a component does work that changes nothing, and how to fix each one.
section: Guides
order: 5
---

Under `sk dev`, the runtime watches how much work each update does and warns when the work is out of proportion to what changed on screen. Each warning names the component and its source file, says what happened and suggests a fix. The bundles `sk build` and `sk publish` make contain none of this code.

## Where they appear

- **The console.** A `[skathi perf]` line, once per distinct cause: the same component, triggered the same way, is reported once per run, not every frame.
- **The devtools Performance tab.** Press **Record interaction**, use the app, stop, and select a frame. The advisories for that frame are listed under its calls, next to the built-in ones such as forced synchronous layout. A frame where the same advisory happened several times shows it once with a count.

## Wasted re-render

Reported when one update of a Vue component re-renders at least 500 elements and fewer than 5% of them change. An element counts as changed when a style, class, attribute or text write, or an inserted or removed child, reached the native tree. Child components that did not need to update are not counted.

The advisory gives the element counts, the time the update took, the reactive values that triggered it (by the name the component holds them under, such as `scrollY` or `sel.col`) and the input event that changed them (`scroll`, `mousedown`, `input` and so on):

```text
[skathi perf] <Grid> (src/components/Grid.vue) re-rendered 3088 elements on scroll and 1 of them changed (7.9 ms).
It was triggered by scrollY. A value that follows the scroll position makes the whole template re-render on every
scroll event: keep headers in view with position: sticky, or move them with a transform set through a template ref
in the scroll handler, so the template does not read the scroll offset.
```

The fix depends on the cause:

- **A value that follows the scroll position.** Chosen when the update came from a `scroll` or `wheel` event, or a value that triggered it has "scroll" in its name. A template that reads `scrollTop`, or a ref set from it, re-renders on every scroll event. Keep headers in view with `position: sticky`, or set the offset with a transform through a template ref inside the scroll handler, so the template does not read it.
- **State that only a few elements show.** Every other cause. A selection, a hover, or the item being edited, read by every row or cell, re-renders all of them when it changes. Draw it as one overlay element positioned over the list, or move the repeated element into a child component that receives only what it shows (`selected`, `editing`) as props. Vue then skips the children whose props did not change. When the component also holds a list of 50 or more items, the advisory adds that virtualizing it bounds the cost.

```vue
<!-- Every cell re-renders when the selection moves -->
<div v-for="c in cols" :key="c" :class="{ selected: c === sel.col && r === sel.row }">{{ cells[r][c] }}</div>

<!-- Only the two cells whose `selected` prop changed update -->
<Cell v-for="c in cols" :key="c" :text="cells[r][c]" :selected="c === sel.col && r === sel.row" />
```

## Long list without virtualization

Reported when a component update re-renders a list of at least 50 items (a `v-for`, or an element with that many children) inside a scroll container, the list holds at least 500 elements, and fewer than 25% of them were in the container's viewport in the last frame. Elements outside the viewport are not painted, but every update of the component still re-renders and diffs all of them.

Render only the items in view: a spacer as tall as the whole list, holding a `v-for` over the visible slice, recomputed from `scrollTop` and the item height. A virtual-list component does the same.

## Host writes per frame

Reported when 1000 or more style, class, attribute and text writes reach the native tree in one frame, with the three components that made the most of them. Writes made while creating elements are not counted, and neither are writes through `element.style`. In a Svelte app, or for DOM code that runs outside a Vue update, the advisory gives the total without naming a component.

When many elements take the same new value, set it once on a shared parent: a class, or a CSS variable the children read (`width: var(--col-3)`). Animate with `transform` or `element.animate()` instead of writing styles on every frame.
