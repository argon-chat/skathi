# vn-elements

Native gpui widgets that are too latency-sensitive to live in JS. Today: one
**single-line text input** with platform IME, selection, clipboard, undo/redo
and a blinking caret.

The crate depends on `gpui` and nothing else.

## Integration contract (as implemented)

```rust
// ---- one-time, at startup, before any window opens -------------------------
pub fn vn_elements::init(cx: &mut App);            // == text_input::init(cx)

// ---- persistent state: one Entity per <input> node -------------------------
pub struct TextInputState { /* content, selection, marked range, undo,
                              blink, FocusHandle, echo ring, ... */ }

impl TextInputState {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self;

    /// Controlled-value sync from the framework (Vue v-model). `write` is the
    /// framework's assignment counter; re-passing the same one is a no-op.
    pub fn sync_value(&mut self, value: &str, write: u32, cx: &mut Context<Self>);

    /// DOM `tabindex`: Some(n) = tab stop at index n, None = out of tab order.
    pub fn set_tab_index(&mut self, index: Option<isize>);

    pub fn text(&self) -> &str;

    // extras the framework needs
    pub fn focus_handle(&self) -> FocusHandle;
    pub fn focus(&self, window: &mut Window, cx: &mut App);
    pub fn is_focused(&self, window: &Window) -> bool;
    pub fn selected_range(&self) -> Range<usize>;
    pub fn marked_range(&self) -> Option<Range<usize>>;
}

impl gpui::Focusable for TextInputState { /* focus_handle(&self, &App) */ }
impl gpui::EntityInputHandler for TextInputState { /* IME, UTF-16 offsets */ }

// ---- per-frame view params (style comes from the CSS engine) ---------------
#[derive(Clone, Debug, Default)]
pub struct TextInputOptions {
    pub placeholder: SharedString,
    pub text_color: Hsla,
    pub placeholder_color: Hsla,
    pub selection_color: Hsla,
    pub caret_color: Hsla,
    pub font_size: Pixels,
    pub disabled: bool,
    pub tab_index: Option<isize>,
}

// ---- per-frame event sinks -------------------------------------------------
pub type OnChange  = Rc<dyn Fn(&str, &mut Window, &mut App)>;
/// Runs BEFORE the widget acts on the keystroke. `false` == preventDefault():
/// no key-binding action and no character insertion for that keystroke.
pub type OnKeyDown = Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App) -> bool>;
pub type OnKeyUp   = Rc<dyn Fn(&KeyUpEvent, &mut Window, &mut App)>;
/// Copy / cut / paste, BEFORE the widget acts. `false` == the event was cancelled.
pub type OnClipboard = Rc<dyn Fn(ClipboardCommand, &mut Window, &mut App) -> bool>;
/// The controlled value (string + write counter) as it stands now. Re-read
/// after `OnKeyDown` / `OnClipboard` let a key or command through, before the
/// widget edits: their handlers may have assigned the field.
pub type ControlledValue = Rc<dyn Fn(&App) -> Option<(String, u32)>>;

#[derive(Clone, Default)]
pub struct TextInputHandlers {
    pub on_change: Option<OnChange>,
    pub on_key_down: Option<OnKeyDown>,
    pub on_key_up: Option<OnKeyUp>,
    pub on_clipboard: Option<OnClipboard>,
    pub controlled_value: Option<ControlledValue>,
}

// ---- the element for one frame --------------------------------------------
pub fn text_input(
    state: &Entity<TextInputState>,
    options: TextInputOptions,
    handlers: TextInputHandlers,
    cx: &mut App,                 // <- deviation, see below
) -> AnyElement;

/// The key_context the bindings are scoped to ("VnTextInput"), exposed so an
/// embedder can add its own bindings against the same context.
pub const KEY_CONTEXT: &str;
```

Per-frame usage:

```rust
state.update(cx, |state, cx| state.sync_value(&value, write, cx));  // 1
let element = text_input(&state, options, handlers, cx);            // 2
```

## Deviations from the requested contract

| # | Deviation | Why |
|---|-----------|-----|
| 1 | `text_input(..., cx: &mut App)` takes an extra context | `on_change` and `disabled` must reach the *entity* — the platform calls `EntityInputHandler::replace_text_in_range` on the entity directly, not through anything the returned element owns — and reaching into an `Entity` needs an `App`. The focus handle for `track_focus` also comes from `state.read(cx)`. |
| 2 | `TextInputState::new(window, cx)` ignores `window` | Nothing in construction needs it today; the parameter is kept so the framework's call site does not change when it does. |
| 3 | `sync_value` takes a write counter and is idempotent *per counter* | It remembers the last **assignment** the framework passed and returns immediately if the counter is unchanged, so vn-tree can call it unconditionally every frame without tracking prop changes itself (and an *uncontrolled* input — one whose `value` prop is never assigned again — is not reset to that prop every frame). Keying on the counter instead of on the string is what lets `v-model` clear the same field twice: the second `""` is a *different assignment* even though it is the same string, and the user has typed in between. |
| 4 | `on_change` fires deferred (`Window::defer`) | It is invoked after the edit's `Entity::update` has finished, so the callback may re-enter this entity — including calling `sync_value` — without the "entity already borrowed" panic. Ordering is preserved, and the echo-ring entry is pushed *synchronously* before the deferral, so an echo can never overtake it. |
| 5 | `disabled` means read-only, not inert | No typing/paste/cut/undo/redo, `accepts_text_input() == false` (so the IME will not target it) and no caret. Focus, selection and copy still work, like a `readonly` HTML input rather than a `disabled` one. Nothing in the contract distinguished the two; flip `read_only` in `TextInputState` if a harder lock is wanted. |
| 6 | Extra public API on the state | `focus_handle`, `focus`, `is_focused`, `selected_range`, `marked_range` — vn-tree needs the handle for autofocus/`.focus()`/focus events, and the ranges are useful for automation/a11y. |
| 7 | Grapheme and word segmentation are re-implemented | The reference used `unicode-segmentation`; "gpui only" rules it out. `src/text_input/boundaries.rs` implements the UAX #29 rules caret motion actually needs (CRLF, combining marks, ZWJ emoji, variation selectors, skin tones, flag pairs) plus Windows-style word motion. Exotic Indic/Brahmic conjuncts fall back to per-`char` motion — finer-grained than the platform, never wrong. Swap in `unicode-segmentation` if that is ever not good enough. |
| 8 | A lone control character is dropped instead of becoming a space | gpui-vue folded *every* newline into a space, which makes a `Return` keypress (which arrives at the input handler as `\r`/`\n` on the platforms that route it there) insert a space. Multi-character text — i.e. a paste — still folds each line break into one space. |
| 9 | Undo history capped at 256 entries | The reference's is unbounded. |
| 10 | Drag-selection listeners live on the window | `div::on_mouse_move` only fires while the pointer is *inside* the element, so the reference (and gpui's own example) stop extending the selection as soon as the drag leaves the input. The element registers window-level move/up listeners during paint instead; they early-out unless a drag is actually in progress. |
| 11 | Double / triple click | Not in the reference: double click selects the word (or the run of whitespace/punctuation) under the pointer, triple click selects everything. |

## What is *not* implemented

* **No tab-order participation.** The wrapper is `track_focus`ed but has no
  `tab_index`, so `Tab` will not move focus into it. Add a `tab_index` field to
  `TextInputOptions` (and `.tab_index(n)` on the wrapper) when vn-tree grows an
  opinion about tab order.
* **No focus/blur events.** Observe them from vn-tree with the focus handle
  (`window.on_focus_changed`, `cx.on_focus`/`on_blur` on the wrapper).
* **No selection API for the framework** beyond reading `selected_range()`.

## What was stripped from the gpui-vue reference

The reference (`research/gpui-vue/crates/gpui-vue-core/src/custom_elements/input.rs`,
1573 lines) is a combined `<input>` + `<textarea>` editor wired into gpui-vue's
own renderer. Removed here:

* **Multiline/textarea entirely** — `shape_text`/`WrappedLine` wrapping, the
  `line_starts` table, `min_rows`/`max_rows` autosize via
  `request_measured_layout`, vertical scrolling and the scroll-wheel handler,
  `Up`/`Down`/`SelectUp`/`SelectDown`, `DocStart`/`DocEnd`, visual-line
  `Home`/`End`, and the `Newline` action. This element shapes exactly one
  `ShapedLine` (as gpui's own example does) and scrolls only horizontally, always
  following the caret.
* **The `Submit` action and every non-`change` event** (`submit`, `keyDown`,
  `keyUp`, `click`, `focus`, `blur`) — `Enter` is deliberately left unbound so a
  single-line input ignores it and the surrounding app can use it.
* **The theme system** (`crate::theme::Theme`, oklch tokens, `theme` prop) —
  replaced by the plain `TextInputOptions` colours.
* **The custom-element plumbing** — `CustomElement`/`CustomElementFactory`,
  `set_prop`/`get_prop`/`supported_props` and their `serde_json` values, the
  `EventCallback`/`emit_event_full` bridge, `apply_styles`, and the
  `crate::text::log_painted_text` debug hook.
* **`read_only` as a prop** — folded into `TextInputOptions::disabled`.
* `unicode-segmentation` (see deviation 7) and tree-sitter/syntax highlighting
  (never part of `input.rs` to begin with).

Kept and ported faithfully: `EntityInputHandler` with UTF-16 offset conversion
and marked-range composition (underlined pre-edit), selection with reversal,
clipboard cut/copy/paste, undo/redo snapshots (one step per composition), caret
blink, placeholder, scroll-follow-cursor, and **the 32-entry echo-suppression
ring** for controlled values.

## Notes for the vn-tree integration

* **Element identity.** Neither the wrapper nor the inner element carries a gpui
  `ElementId`; identity lives entirely in the `Entity<TextInputState>` the
  framework keeps in its side map. Rebuilding the element every frame is free,
  and giving the wrapper an id is unnecessary (but harmless, if vn-tree wraps it).
* **Call order.** `sync_value` first, then `text_input(...)`, both from the
  framework's render — never from *inside* a `state.update` closure (that would
  re-enter the entity). Do not call `sync_value` between prepaint and paint.
* **`on_change` → `sync_value` round trip.** The value handed to `on_change` is
  exactly what a later `sync_value` must be able to match for the echo ring to
  swallow it: pass it back verbatim (no trimming, no re-encoding). Values that
  do *not* match are treated as external and hard-reset content, caret **and
  undo history** — so do not push a normalised/formatted value back on every
  keystroke unless that reset is what you want.
* **Line breaks.** Content is always newline-free. `sync_value("a\nb")` stores
  `"a b"`, so `text()` can differ from the value that was pushed in; the
  framework will see the folded value on the next `on_change`.
* **Focus.** Clicking anywhere in the element focuses it and places the caret.
  The element fills its content box (`w_full h_full`, line centred), but clicks
  on the *padding/border* of a wrapper vn-tree styles itself land on that
  wrapper, not on us — attach `on_mouse_down` there and call
  `state.read(cx).focus_handle()` + `window.focus(&handle, cx)` if you want the
  HTML behaviour. For autofocus, call `TextInputState::focus` once the entity
  exists.
* **`init(cx)` is mandatory** before the first input is used, otherwise no key
  bindings exist (typing would still work through the platform text path, but
  arrows, backspace, ctrl-a/c/x/v/z/y and friends would not).
* **Repaints.** The state entity is read during prepaint, so `cx.notify()` from
  a key/mouse handler invalidates the window even though the state is not a
  `Render` view. While focused, the caret blink schedules a repaint every 500 ms.

## Key bindings (installed by `init`)

`ctrl` is the primary modifier everywhere except macOS, where it is `cmd`; the
word-wise modifier is `ctrl` (macOS: `alt`).

| Keys | Action |
|------|--------|
| `left` / `right`, `shift-…` | move / extend by grapheme cluster |
| `ctrl-left` / `ctrl-right`, `shift-…` | move / extend by word |
| `home` / `end`, `shift-…` | move / extend to start / end |
| `backspace`, `delete` | delete grapheme or selection |
| `ctrl-backspace`, `ctrl-delete` | delete word |
| `ctrl-a` / `-c` / `-x` / `-v` | select all / copy / cut / paste |
| `ctrl-z`, `ctrl-y`, `ctrl-shift-z` | undo, redo, redo |
| `escape` | collapse the selection (propagates when there is none) |
| `enter` | *unbound* — ignored by a single-line input |
| macOS: `cmd-left/right`, `cmd-backspace/delete`, `ctrl-cmd-space` | line start/end, delete to start/end, character palette |

## Attribution

Adapted from **gpui-vue** (`crates/gpui-vue-core/src/custom_elements/input.rs`,
Apache-2.0), which is itself derived from:

* the native renderer in [remorses/gpuix](https://github.com/remorses/gpuix),
  commit `b8bbff4ef0dc00dc24c3d3f3ca47818e1be15c69`, which declares the Apache
  License 2.0; and
* [zeronsh/comet](https://github.com/zeronsh/comet) (MIT) — gpui-vue notes that
  its caret blinking is ported from Comet's `crates/ui/src/composer.rs`.

See gpui-vue's `THIRD_PARTY_NOTICES.md`
(`research/gpui-vue/THIRD_PARTY_NOTICES.md`) for the upstream notices.

Structure and current-API usage also follow gpui's own
`crates/gpui/examples/input.rs` (Apache-2.0, zed-industries/zed) — the authority
whenever it and the older gpui-vue code disagree.

Each ported file repeats this notice in its header.
