---
title: "<input> and <textarea>"
label: "<input> and <textarea>"
metaTitle: "<input> and <textarea>: Skathi's native text fields"
description: How Skathi's native text editor behaves in <input> and <textarea>, which types and attributes it reads, IME on each platform, undo, the clipboard, keys, events, styling, and what a browser has that it does not.
section: Elements
order: 11
---

Every `<input>` that is not a checkbox, radio, range, color or file input, and every `<textarea>`, is Skathi's own text editor. It is written in Rust on gpui's text input: it lays out and draws the text, the caret and the selection itself, and takes keyboard and IME input from the operating system directly. `v-model` and `bind:value` work as in a browser.

The other input types are on [Form controls](/elements/form-controls/).

```vue
<script setup lang="ts">
import { ref } from "vue";

const name = ref("");
const notes = ref("");
</script>

<template>
  <input v-model="name" placeholder="Name" />
  <textarea v-model="notes" rows="4" placeholder="Notes" />
</template>
```

## Types

`text`, `search`, `url`, `tel`, `email`, `password` and `number` are all a single-line text field. `password` draws one bullet per character and refuses copy and cut; switching `type` between `password` and `text` shows and hides the value. `number` adds `valueAsNumber`, but there is no spinner and no input filtering.

`date`, `time` and the other types without a native control here are plain text fields too, without a picker. `hidden`, `submit`, `button`, `reset` and `image` are also drawn as text fields: use a `<button>`, and keep hidden values in state rather than in the tree.

A `<textarea>` is the same editor with several lines. Its initial value comes from its text content.

## Attributes

| Attribute              | Behavior                                                                                                                                 |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `value`                | The text. Setting it from script replaces the text and clears the undo history.                                                          |
| `placeholder`          | Shown while empty. In a single-line field, a line break in it shows as a space.                                                          |
| `disabled`, `readonly` | Both stop editing: no typing, paste, cut, undo or IME, and no caret. The field can still be focused, and text in it selected and copied. |
| `rows`                 | A `<textarea>`'s minimum height in lines, 2 by default. CSS `height` wins.                                                               |
| `tabindex`             | As on any element.                                                                                                                       |

`maxlength`, `minlength`, `pattern`, `min`, `max`, `step` and `required` are reflected as properties but not enforced, and no constraint validation runs: there is no `checkValidity()` or `validity`, and a form's `submit` fires whatever the fields hold. Validate in your own code.

`autofocus` does nothing; focus the field with `el.focus()` once it is mounted. `cols` and `wrap` are ignored: a textarea always wraps lines to its width. `spellcheck`, `autocorrect`, `autocapitalize`, `enterkeyhint` and `inputmode` are passed to the platform's text input, which ignores them on desktop: there is no spell-check underline. A `disabled` `<fieldset>` does not disable the text fields inside it.

## Keys

| Keys                                         | Action                                                                   |
| -------------------------------------------- | ------------------------------------------------------------------------ |
| Ctrl+Z / Cmd+Z                               | Undo                                                                     |
| Ctrl+Y, Ctrl+Shift+Z / Cmd+Shift+Z           | Redo                                                                     |
| Ctrl+C, Ctrl+X, Ctrl+V / Cmd+C, Cmd+X, Cmd+V | Copy, cut, paste                                                         |
| Ctrl+Insert, Shift+Delete, Shift+Insert      | Copy, cut, paste, on Windows and Linux                                   |
| Enter                                        | A new line in a `<textarea>`; in an `<input>`, fires the form's `submit` |
| Up, Down in an `<input>`                     | Move to the start or the end                                             |
| Escape                                       | Collapses the selection; with none, the key goes on to the app           |
| Tab                                          | Moves focus; it is never typed                                           |
| Ctrl+Cmd+Space                               | The character palette, on macOS                                          |

Undo keeps 256 steps. Each edit is its own step; a whole IME composition is one.

With the mouse, a double-click selects a word, a triple-click selects everything, Shift+click extends the selection, and a drag that leaves the field keeps selecting. There is no context menu: right-click does nothing unless you handle `contextmenu` and draw a menu yourself.

The clipboard carries plain text. Pasting into a single-line field turns line breaks into spaces.

## IME

Composition input works on every platform, with the candidate window placed at the caret, also under CSS transforms:

| Platform | Through                                    |
| -------- | ------------------------------------------ |
| Windows  | IMM32                                      |
| macOS    | `NSTextInputClient`                        |
| Linux    | `zwp_text_input_v3` on Wayland, XIM on X11 |

`compositionstart`, `compositionupdate` and `compositionend` fire in Chromium's order. `v-model` waits for the composition to end before it updates the model. Setting `value` during a composition ends it and still fires `compositionend`.

## Events

| Event                                                     | Notes                                                                                |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `keydown`, `keyup`                                        | `keydown` runs before the editor acts on the key, and `preventDefault()` stops it.   |
| `beforeinput`                                             | Cancelable, except during a composition. Not fired for an edit that changes nothing. |
| `input`                                                   | With `inputType` and `data`.                                                         |
| `compositionstart`, `compositionupdate`, `compositionend` | See [IME](#ime).                                                                     |
| `copy`, `cut`, `paste`                                    | Fire first; `preventDefault()` cancels the action.                                   |
| `focus`, `blur`                                           |                                                                                      |
| `select`                                                  | After `select()`.                                                                    |

**`change` never fires on a text field.** `v-model.lazy` updates on every keystroke instead and prints an advisory. To act when the user is done, use `blur` or Enter; [Vue support](/guides/vue-support/) shows the pattern.

## Selection from script

`select()` focuses the field and selects all of its text. `selectionStart`, `selectionEnd`, `selectionDirection`, `setSelectionRange()` and `setRangeText()` do not exist: a script cannot read or place the caret.

## Styling

The field is styled like any box: `font`, `color`, `padding`, `border`, `background` and `text-align` apply.

- [`::placeholder`](/css/pseudo-placeholder/): its `color` applies; the default is the text color at 45 % opacity.
- [`::selection`](/css/pseudo-selection/): its `background` applies, its `color` does not.
- The caret is 2 pixels wide, blinks every 500 ms and has the text color. [`caret-color`](/css/caret-color/) is not supported.
- A single-line field scrolls sideways to keep the caret in view. A `<textarea>` scrolls with the wheel and draws its own thin scrollbar.
- [`field-sizing`](/css/field-sizing/) is not supported, and [`resize`](/css/resize/) shows no handle.

## Compared with the browser

- No `change` event on text fields.
- No selection API beyond `select()`, and no context menu.
- No constraint validation; `maxlength` is not enforced.
- No spell checking, no `autofocus`, no date or time pickers, no number spinner.
- Disabled fields stay focusable.

## See also

- [Form controls](/elements/form-controls/): checkboxes, radios, range, color, `<select>` and files.
- [Vue support](/guides/vue-support/) and [Svelte support](/guides/svelte-support/): `v-model` and `bind:value`.
- [`InputEvent`](/web-platform/input-event/) in the web-platform reference.
