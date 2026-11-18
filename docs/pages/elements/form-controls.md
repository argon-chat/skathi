---
title: Form controls
label: Form controls
metaTitle: "Form controls in Skathi: checkbox, radio, range, color, select and file"
description: How Skathi draws checkboxes, radios, range sliders, color inputs, <select> and file inputs natively, which keys and events each has, how to style them, and how forms submit.
section: Elements
order: 12
---

Skathi draws its own form controls, styled to match Chromium on Windows, and opens the operating system's dialogs where a browser would: the file picker and the color picker. Text fields are on their own page: [`<input>` and `<textarea>`](/elements/text-fields/).

Each control works with `v-model` in Vue and `bind:value`, `bind:checked` or `bind:group` in Svelte. [`appearance: none`](/css/appearance/) removes a control's default look, and [`accent-color`](/css/accent-color/) tints checkboxes, radios and sliders.

## Checkboxes and radios

`<input type="checkbox">` and `<input type="radio">` are native. A click, Space or a click on their `<label>` checks them. Radio buttons with the same `name` in the same form are a group, and the arrow keys move through it. They fire `click`, then `input` and `change`, and `preventDefault()` on the click undoes the change. `checked`, `defaultChecked`, `indeterminate` and `disabled` work, and so do `:checked`, `:indeterminate`, `:disabled` and `:focus-visible`.

[Vue support](/guides/vue-support/) and [Svelte support](/guides/svelte-support/) cover binding them.

## Range

`<input type="range">` is a native horizontal slider, 129 × 16 pixels unless CSS sizes it.

- `min`, `max` and `step`, including `step="any"`, apply.
- The arrow keys, Page Up, Page Down, Home and End move it, and so does dragging.
- `input` fires while it moves, `change` when a drag or a key press commits.
- `stepUp()` and `stepDown()` work.
- Style it with `::-webkit-slider-thumb` and `::-webkit-slider-runnable-track` (or the `-moz-range-` spellings), `accent-color`, and `appearance: none`.

There is no vertical slider: the track always runs along the width.

## Color

`<input type="color">` is a swatch, 50 × 27 pixels by default. A click opens the system's color picker: `ChooseColor` on Windows, `NSColorPanel` on macOS, and zenity or kdialog on Linux. A pick sets `value` and fires `input` and `change`; cancelling fires nothing. As in browsers, the picker needs a user gesture, and `showPicker()` opens it from your own button's click.

`value` is a lowercase `#rrggbb`. Other hex forms are normalized to it; any other CSS color, such as `rgb()` or a name, becomes `#000000` with an advisory. `::-webkit-color-swatch` and `::-webkit-color-swatch-wrapper` style the swatch.

## Select

`<select>` draws a closed box like Chromium's on Windows: the selected option's label, a chevron, and the width of its widest option. `appearance: none` removes the chevron and the padding.

The list opens **inside the window**, drawn by Skathi, not as an OS menu. It opens under the box, or above it when there is no room, stays inside the window, shows up to 20 rows before it scrolls, and closes when the window loses focus. Its colors are a fixed light or dark palette; only the select's own solid background carries over.

- Alt+Up, Alt+Down, F4, Enter or Space opens the list.
- While it is closed, the arrow keys change the value, and typing selects the first option that starts with what was typed (the search restarts after a one-second pause).
- `<optgroup>` labels are bold and their options indented. Disabled and `hidden` options are honored.
- A pick fires `input`, then `change`.

`multiple` and `size` keep the drop-down: the box opens the list and each pick selects one option, with an advisory. The selection model behind it works, so `selectedOptions` and a `v-model` bound to an array do. `appearance: base-select` is not supported.

## File

`<input type="file">` opens the operating system's file dialog: `IFileOpenDialog` on Windows, `NSOpenPanel` on macOS, and the XDG desktop portal on Linux. `accept`, `multiple` and `webkitdirectory` are passed to it.

- A choice sets `files`, then fires `input` and `change`. Dismissing the dialog, or choosing the same files again, fires `cancel`.
- The dialog needs a user gesture. A `<label for>` pointing at the input opens it, and `showPicker()` opens it from your own button's click.
- `value` reads `C:\fakepath\` and the first file's name, as in browsers. Only `""` can be assigned.
- The button and its text copy Chromium's English strings. [`::file-selector-button`](/css/pseudo-file-selector-button/) styles the button; most apps hide the input and open it from a button of their own.

[`<input type=file>` in the web-platform reference](/web-platform/input-type-file/) has examples, and [File System Access](/web-platform/file-system-access/) opens files and folders with handles you can write to.

## Labels

A `<label>` finds its control by `for`, or as its first labelable descendant. A click on it focuses a text field, or clicks the control: it checks a checkbox, opens a file dialog.

## Forms

A `<button type="submit">` and Enter in a text field fire the form's `submit` event; a reset button fires `reset`. `requestSubmit()` fires `submit` too. That is all a form does: there is no page to navigate to, so a `submit` you do not cancel only prints an advisory. Handle `submit` with `preventDefault()` (`@submit.prevent` in Vue) and send the data yourself.

No constraint validation runs, and a reset does not restore the fields' values. `form.submit()` fires no event.

## Not implemented

`<progress>` and `<meter>` draw nothing, `<dialog>` has no `showModal()`, and `<details>` does not toggle. See [What is not here](/elements/overview/#what-is-not-here).

## See also

- [`<input>` and `<textarea>`](/elements/text-fields/).
- [`appearance`](/css/appearance/) and [`accent-color`](/css/accent-color/) in the CSS reference.
- [Vue support](/guides/vue-support/) and [Svelte support](/guides/svelte-support/).
