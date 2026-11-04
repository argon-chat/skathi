# `<lottie>` integration contract

What the host (`vn-tree`) has to do to turn this crate into a `<lottie>` element.
Nothing here is aspirational — every entry names an API that exists in
`vn-lottie` today. Written to be mechanical; if a step needs a judgement call,
that is a bug in this document.

> **Status: IMPLEMENTED** (2026-08-26, milestone 10 integration track).
> `crates/vn-tree/src/lottie.rs` is the element layer, `render.rs::build_lottie`
> builds the box, `vn_tree::frame_tick` drives the pump from `vn-app`. Where the
> element layer deliberately differs from this document, the difference is
> recorded in [§8 Element-layer decisions](#8-element-layer-decisions) — **the
> element layer wins**; the sections above it describe the *crate's* semantics.
> Report: `research/reports/m10/integration.md`.

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
transition does. Extend the OR-chain in `crates/vn-app/src/lib.rs:160`:

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

## 8. Element-layer decisions

Where the shipped `<lottie>` element differs from §1–§7 above. §1–§7 describe
the **crate**; this section describes the **element**, and for anything written
in a Vue template the element is what applies. All of it lives in
`crates/vn-tree/src/lottie.rs`, which carries the same list in its module docs.

### 8.1 `autoplay` and `loop` default to `false`

§1 and §5 say `true`, following lottie-web's `loadAnimation()`. The element
follows the **`<lottie-player>` web component** instead, which defaults both to
`false` — a genuine split in the ecosystem, decided on the grounds that
`<lottie>` is *markup*, and the markup precedent is the web component. Pasting a
`<lottie-player>` tag into a vue-native template therefore behaves the way it did
in the browser; a `lottie-web` user gets the other surprise, which is why this is
documented rather than left implicit.

Overridden at the element layer, **not** in the crate: a Rust embedder driving
`LottieState` directly is in lottie-web's world, so `LottieState::load` still
comes back playing and looping.

### 8.2 `loopComplete` is suppressed on the completing boundary

§5 offers the one-line fix and §2 calls the extra event "the one deliberate
divergence". It is now suppressed, and slightly more broadly than §5 proposes:
the filter is *"drop the `LoopComplete` whose count equals `advance.loop_count`
when `advance.completed` is set"*, which covers `LoopMode::Count` as well as
`LoopMode::None`. lottie-web fires `complete` alone at the end of a counted loop
too.

### 8.3 `:loop="n"` maps to `Count(n + 1)`

§1 says `:loop="3"` → `Count(3)`, "which plays 4 times". The crate's
`Count(n)` actually plays exactly **n** times (`keep_going = loops_done < count`,
and `loops_done` is incremented first), so the mapping needs the `+ 1` to mean
what lottie-web's `loop: n` means — *n repeats*, i.e. n + 1 plays. Combined with
§8.2 the event stream is then lottie-web's exactly: `loopComplete` ×n, then
`complete`.

Left as a mapping rather than a crate change so the crate's 36 tests keep
describing the crate. If `LoopMode::Count` is ever redefined to mean "repeats",
this mapping must lose its `+ 1` on the same commit.

The `complete` payload's `c` (loops completed) is `loop_count - 1` when the tick
completed, so it equals lottie-web's `playCount`: `0` for a non-looping play,
`n` for `:loop="n"`.

### 8.4 `autoplay` is edge-triggered, not load-only

`<lottie-player>` reads `autoplay` at load and exposes `.play()` / `.pause()`
methods for everything after. We have no element methods, so a *change* to the
prop is treated as a play/pause command — which is what makes
`:autoplay="isPlaying"` usable as the transport control markup needs it to be.
A deliberate, documented superset: the load-time behaviour is identical, and a
value that never changes never issues a command (so a paused animation is not
restarted every frame).

### 8.5 Event names accept both spellings

The events are registered under lottie-web's own camelCase names (`complete`,
`loopComplete`, `enterFrame`), so `e.type` reads as it does in the browser.
`compiler-dom` emits the case-preserving `on:loopComplete` prop for a custom
element, which lands on the right name directly; the hyphenated spelling
(`loop-complete`, what `onLoopComplete` would hyphenate to) and the all-lowercase
one are accepted as aliases and folded onto the same bucket. All three therefore
work from a template, and `addEventListener("loopComplete")` works from JS.

### 8.6 The box, and `object-fit`

Implemented as §6 specifies — CSS `width`/`height` win, one dimension derives the
other from the intrinsic aspect ratio, neither means the intrinsic canvas size —
with one addition: **a relatively sized box** (`width: 100%`, a flex child) is not
resolvable at build time, because gpui lays out after the element tree is built.
Those rasterise at the rectangle the node was *painted at in the previous frame*
(a new `PREV_BOUNDS` side map next to the devtools `BOUNDS` one), falling back to
the computed pair on the very first frame. The crate re-rasterises on any size
change, so the picture is correct from frame two with no layout pass of our own.

`object-fit` is wired: it comes off the cascade (`ComputedStates::extras`, the
same source `<img>` uses), with the `objectfit` attribute as a presentational
fallback, defaulting to `contain` per §6 — **not** to the CSS initial value
`fill` that `<img>` uses, because this element mirrors `<lottie-player>`'s
`preserveAspectRatio: "xMidYMid meet"`.

`grayscale` is always `false`: the cascade does not carry `filter` yet. One line
in `build_lottie` the day `Extras` grows the property.

### 8.7 `segment` takes a string

`:segment="[10, 40]"` cannot reach the host: `patchProp` drops non-primitive prop
values with its own advisory. `segment="10,40"` (or `"10 40"`) is the supported
spelling.

### 8.8 A detached `<lottie>` keeps its clock running

§3 keys the side map on node id and §4 on `needs_frame()`, neither of which knows
about reachability — so a `<KeepAlive>`-cached page's animation keeps ticking
while it is parked in the detached storage container. That is deliberate and it
is the browser behaviour: a `<lottie-player>` in a detached subtree never cancels
its rAF loop. Nothing is painted meanwhile and no raster is produced (the element
is not built), so the cost is the transport arithmetic.

### 8.9 GC drops go through a queue

§3 notes `gc_node_state` has no `Window`. It queues the tiles instead: `gc`
retires the node's tile into its window's drop queue, which that window's next
frame drains into `App::drop_image`. Same for a `src` swap, which retires the
whole previous state.

### 8.10 The load and the raster are off the UI thread

§3, §4 and §7 describe the crate's `LottieState`, which loads in the call that
creates it and rasterises inside the build. The element does neither on the UI
thread (m26, `research/reports/m26/stickers-1.md` "L"):

- **The UI thread keeps only the transport**, `vn_lottie::Transport`: the
  playhead, the props, `advance()` and its events. It is `LottieState`'s own
  transport, split out, so the event stream, the sub-frame playhead and §8.1–8.4
  are unchanged.
- **The load is asynchronous.** A file is read and parsed, and the engine
  loads, on the `vn-lottie` worker thread; an `http(s)` `src` is fetched on
  gpui's HTTP client first (the element used to refuse one). Until the load
  lands the element is an empty box, as an `<img>` is before it decodes, and
  its intrinsic size is unknown. The transport starts on the tick the load
  lands, with the props the node has then, so `autoplay` decides there whether
  it plays (§8.4). A `src` that cannot be resolved still fails at once, with
  the same advisory.
- **The raster trails the transport by at most one vsync.** Each tick asks the
  worker for the frame at the playhead, at the box's device-pixel size
  (`vn_lottie::raster_size_for`, the size `render_frame` draws), for a node
  that can be seen. The next tick writes the frame into the node's `LiveTile`;
  in place, that is a present and no build. When the worker falls behind, the
  newest frame asked for wins. Events go out on the transport's tick, as
  before, so an `enterFrame` handler sees the frame one vsync before it shows.
- **Straight alpha.** `vn_lottie::Animation::render_at` draws straight-alpha
  BGRA, which is what gpui blends; the premultiplied raster darkened
  antialiased edges.
- **A node nobody can see draws nothing**: off screen, clipped away by an
  ancestor, in a hidden page or a detached subtree (§8.8). Its tile keeps the
  last frame it showed.
- **Not a reason to repaint.** §4's OR-chain is gone from the host:
  `lotties_animating()` no longer makes vn-app draw every frame. The pump ticks
  every frame anyway, and the tick asks for a draw only when a present cannot
  show a frame (a tile's first frame, a new size, a tile that moved).
- **Cacheable.** `<lottie>` is no longer one of `crate::cache`'s exclusions: a
  frame written in place is shown by a replayed sprite, and a first frame, a
  new size or a tile that moved in the atlas touches the node.
- The `webworkers` prop's advisory now says rasterisation always runs on a
  worker.
