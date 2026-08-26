# `<lottie>` integration contract

What the host (`vn-tree`) has to do to turn this crate into a `<lottie>` element.
Nothing here is aspirational — every entry names an API that exists in
`vn-lottie` today. Written to be mechanical; if a step needs a judgement call,
that is a bug in this document.

```html
<lottie src="sticker.tgs" autoplay loop speed="1.5" @complete="onDone" />
```

## 1. Element and props

`<lottie>` is a **leaf** element: it never has children, and its subtree in the
retained tree is always empty. It participates in layout and the cascade like
any other box.

### Prop → state mapping

| prop | type | default | maps to | notes |
| --- | --- | --- | --- | --- |
| `src` | string | — | `LottieState::load(&bytes)` | **The host resolves this, not the crate.** Path, `data:` URI, bundled asset — whatever the host's loader already does for `<img>`. The crate only ever takes `&[u8]`. |
| `autoplay` | bool | **`true`** | `play()` / `pause()` after load | See §5 for why `true`. |
| `loop` | bool \| number | **`true`** | `set_loop_mode(LoopMode::None \| Loop \| Count(n))` | `loop` → `Loop`; `:loop="false"` → `None`; `:loop="3"` → `Count(3)`, which plays 4 times (lottie-web's numeric `loop` counts *repeats*). |
| `speed` | number | `1` | `set_speed(f32)` | Clamped `>= 0`. `0` is a pause, not an error. |
| `direction` | `1` \| `-1` | `1` | `set_direction(Direction::Forward \| Reverse)` | `-1` also makes `stop()` rewind to the out point. |
| `mode` | `"normal"` \| `"bounce"` | `"normal"` | `set_play_mode(PlayMode::Normal \| Bounce)` | `bounce` ping-pongs instead of jumping back. |
| `segment` | `[in, out]` | — | `set_segment(Some((f32, f32)))` | Reversed pairs are normalised, out-of-range ends clamped, and the playhead is pulled inside. Absent/`null` → `set_segment(None)`. |
| `objectfit` | `contain`\|`cover`\|`fill`\|`none`\|`scale-down` | `contain` | `LottieOptions::object_fit` | Also settable via the CSS `object-fit` property; see §6. |

Props are re-read every frame from the retained tree, exactly like
`TextInputOptions`. Only *changes* should be pushed into the state — calling
`set_speed` with the same value every frame is harmless, but `play()`/`pause()`
must be edge-triggered off `state.is_playing()` or a paused animation will
restart itself every frame.

### Not implemented — emit a compat advisory if used

`renderer` (always software raster), `background` (use CSS `background-color` on
the element), `controls`, `hover`, `intermission`, `count`,
`preserveAspectRatio` (use `object-fit`), `webworkers`, `subframe` (always on —
the playhead is continuous, never snapped to source frames). Route these through
`warn_compat` in `render.rs` the way unsupported CSS already is.

## 2. Events out

Emitted from `Advance::events()`, which yields them in lottie-web's firing
order. Names are lottie-web's, so `@complete` / `@loopComplete` / `@enterFrame`
port from the browser unchanged.

Add to the `EV_*` block in `vn-tree/src/tree.rs`, continuing from `EV_SCROLL = 10`:

| event | const | `EventPayload` | fires |
| --- | --- | --- | --- |
| `complete` | `EV_COMPLETE = 11` | `a` = final frame, `b` = total frames, `c` = loops completed | Playback finished and will not resume. Once per completion. |
| `loopComplete` | `EV_LOOPCOMPLETE = 12` | `a` = frame at the boundary, `b` = total frames, `c` = loop count (1-based) | One pass over the active segment ended. Never fires for `loop=false` in lottie-web; here it fires once immediately before `complete`, which is the one deliberate divergence — see §5. |
| `enterFrame` | `EV_ENTERFRAME = 13` | `a` = current frame, `b` = total frames, `c` = frames elapsed in segment, `d` = progress `0..1` | Every frame the playhead moves. **High frequency — gate on the listener bit** (`node.listeners & (1 << EV_ENTERFRAME)`) and do not build the payload at all when nothing is listening. |

The crate emits nothing else. `dataReady`/`DOMLoaded`/`error`/`destroy` are
load-lifecycle events the *host* owns, since the host is what resolves `src`;
`segmentStart` and `drawnFrame` are not implemented.

Delivery goes through the existing path — `js_dispatch(id, EV_*, &payload, window, cx)`.
All three are fire-and-forget: none is cancellable, so the `bool` return is ignored.

## 3. Lifecycle

State lives in a side map keyed by node id, alongside `INPUT_STATES` in
`vn-tree/src/render.rs`:

```rust
static LOTTIE_STATES: RefCell<HashMap<u32, LottieState>> = RefCell::new(HashMap::new());
```

Plain values, not `gpui::Entity` — `LottieState` needs no `Context`, no focus
handle and no subscriptions, so it is closer to `SCROLL_HANDLES` than to
`INPUT_STATES`.

**Creation** is keyed on `src`: the entry is (re)built when the node has no
state yet or when `src` changed. Keep the resolved `src` next to the state so
the comparison is cheap.

**GC** — in `gc_node_state()`, and this one is not a plain `retain`: dropping a
`LottieState` leaks its atlas tiles. Hand them back first:

```rust
LOTTIE_STATES.with(|m| m.borrow_mut().retain(|&id, state| {
    if live(id) { return true; }
    for image in state.take_all_images() { window.drop_image(image).ok(); }
    false
}));
```

`gc_node_state` currently has no `Window`; either thread one in or queue the
images for the next frame that does.

## 4. Driving the frame pump

`advance()` must be called **once per frame per node, before building the
element**, with the wall-clock delta since the last frame. `vn-app`'s
`pump_frame` is the place — it already runs once per frame and already owns the
`Instant`-based cadence the CSS animator uses.

An animating Lottie must keep frames coming the same way a running CSS
transition does. Extend the OR-chain in `crates/vn-app/src/main.rs:160`:

```rust
if vn_tree::flush_changes() || vn_tree::animating() || vn_tree::lotties_animating() {
```

where `vn_tree::lotties_animating()` folds over the side map:

```rust
pub fn lotties_animating() -> bool {
    LOTTIE_STATES.with(|m| vn_lottie::any_playing(m.borrow().values()))
}
```

`LottieState::needs_frame()` is the per-state predicate (`playing && !finished
&& speed > 0`), and `vn_lottie::any_playing` is the ready-made fold. A paused or
completed animation returns `false`, so an idle window goes back to sleep —
`advance()` on a paused state is a couple of branches and allocates nothing, so
calling it unconditionally on every node is free.

## 5. Compat notes

**`autoplay` and `loop` default to `true`.** This is lottie-web's
`loadAnimation()` default. The `<lottie-player>` / `<dotlottie-player>` web
components default *both* to `false` — a genuine split in the ecosystem. We
follow lottie-web because that is what a Vue app using `lottie-web` directly (or
`vue3-lottie`, which wraps it) already behaves like, so porting a component does
not silently change what it does. **A `<lottie-player>` user porting markup will
see animations play that did not play in the browser**; that is worth a note in
the docs, not an advisory, since it is a documented deliberate choice.

**`loopComplete` before `complete` when not looping.** lottie-web fires only
`complete` for a non-looping animation. Here the final pass also produces a
`loopComplete`. Suppress it at the dispatch site if exact parity matters:

```rust
if advance.loops_completed > 0 && state.loop_mode() != LoopMode::None { /* dispatch */ }
```

**Feature coverage.** The renderer draws a large but not complete subset of
Lottie. `LottieState::load` returns `LottieError::Unsupported` with one
`advisories()` line per unsupported feature; route those through `warn_compat`
so an animation that cannot render says *why* instead of silently drawing
nothing. `research/reports/m10/lottie.md` has the coverage matrix.

**Successful loads can also carry advisories.** `LottieState::advisories()` is
non-empty when the compatibility pass had to rewrite the document (terminator
keyframes, solid layers). These are informational, not errors.

## 6. CSS interaction

**Size.** CSS `width`/`height` win outright. With neither, the box takes the
animation's intrinsic size — `meta().width` / `meta().height`, the Lottie
canvas — which is the `<img>` rule. With one of the two, the other follows from
the intrinsic aspect ratio.

`LottieOptions::size` is the resulting **content box in logical (CSS) pixels**,
and `LottieOptions::scale` is `window.scale_factor()`. The two are multiplied to
get the physical raster size, so a 96×96 box on a 150% display rasterises at
144×144. Never pass physical pixels as `size`; never pass `1.0` as `scale` on a
HiDPI display.

**`object-fit`** behaves as in CSS, defaulting to `contain` — which also matches
`<lottie-player>`'s `preserveAspectRatio: "xMidYMid meet"`. Under `contain` the
raster is exactly 1 texel per physical pixel; under `cover` it is upscaled by
the overflow ratio. `object-position` is not implemented (gpui's image element
centres).

**Background.** The raster is transparent where the animation is. CSS
`background-color`/`background-image` on the element show through, so
`<lottie-player background="#fff">` becomes `style="background: #fff"`.

**`prefers-reduced-motion`.** Not handled by the crate. The host already tracks
this for the CSS animator (`set_reduce_motion`); the natural policy is to
`pause()` and `seek_progress(0.0)` playing Lotties when it turns on, and it
should be applied at the same site.

## 7. Sketch

```rust
fn build_lottie<V: 'static>(
    t: &Tree, id: u32, node: &Node, window: &mut Window, cx: &mut Context<V>,
) -> AnyElement {
    let mut states = LOTTIE_STATES.borrow_mut();
    let Some(state) = states.get_mut(&id) else {
        return div().into_any_element(); // still loading, or load failed
    };

    // Props, every frame.
    state.set_speed(t.prop(id, "speed").and_then(|v| v.parse().ok()).unwrap_or(1.0));
    state.set_loop_mode(loop_mode_from(t.prop(id, "loop")));
    let autoplay = t.prop(id, "autoplay").is_none_or(|v| v != "false");
    if autoplay && !state.is_playing() && !state.is_finished() { state.play(); }

    // The tick was done once for the whole tree in `pump_frame`; here we only
    // read the result that was stashed for this node.
    let (size, fit) = box_from_cascade(node, state.meta());
    let options = LottieOptions {
        size,
        scale: window.scale_factor(),
        object_fit: fit,
        grayscale: node.computed_grayscale(),
    };
    vn_lottie::lottie(state, &options, window, cx)
}
```

`vn_lottie::lottie` rasterises (from cache when nothing moved) and releases
retired atlas tiles. **Do not call `LottieState::render_frame` directly** — it
does not release them, and at 60fps that is one leaked texture per frame.
