# vn-css

The CSS layer: `lightningcss` parses, `parcel_selectors` matches, the cascade
folds matched rules into per-state `gpui::StyleRefinement`s, and the **animator**
runs CSS `transition`s over the result.

```
src/lib.rs         Engine::compile / compute — the cascade, ComputedStates
src/element.rs     parcel_selectors::Element over the MatchDom trait
src/mapper.rs      lightningcss::Property -> gpui::StyleRefinement
src/transition.rs  transition parsing + the timing model (easing, specs)
src/animator.rs    Animator: diff -> tracks -> per-frame overlay refinement
```

---

## 1. The transition contract (what vn-tree / vn-app must wire)

The animator is a plain struct. It owns no globals, no thread-locals and no
clock — the host decides when a restyle happened, what "now" is, and when a node
died. Three call sites, in this order.

### 1.1 After the cascade — `Animator::on_computed`

```rust
pub fn on_computed(
    &mut self,
    node: u32,
    old: Option<&ComputedStates>,
    new: &ComputedStates,
    now: std::time::Instant,
)
```

`ComputedStates` now carries the folded transition list:

```rust
pub struct ComputedStates {
    pub base: Rc<StyleRefinement>,
    pub hover: Option<Rc<StyleRefinement>>,
    pub active: Option<Rc<StyleRefinement>>,
    pub transitions: Rc<Vec<TransitionSpec>>,   // <- new
}
```

`Tree::restyle` (crates/vn-tree/src/tree.rs) already produces
`Vec<(u32, ComputedStates)>` and then writes each into `node.computed`. The
animator hooks the write-back:

```rust
// tree.rs, step 2 of restyle(), replacing the plain store loop
let now = std::time::Instant::now();       // ONE instant for the whole pass
for (id, states) in computed {
    let Some(n) = self.node_mut(id) else { continue };
    // vn-tree recomputes EVERY node on every restyle, so most nodes are
    // unchanged. `ComputedStates: PartialEq` (gpui's `StyleRefinement`
    // derives PartialEq through the refineable macro — verified at
    // gpui/src/style.rs:178, `#[refineable(Debug, PartialEq, ...)]`), so a
    // value compare is the cheap "did this node actually change?" test.
    let changed = n.computed.as_ref() != Some(&states);
    if changed {
        animator.on_computed(id, n.computed.as_ref(), &states, now);
        n.computed = Some(states);
    }
}
for d in animator.take_diagnostics() {
    eprintln!("[css transition] {d}");
}
```

Rules the animator relies on:

* **One `Instant` per restyle pass.** Every node that changed in the same pass
  must get the same `now`, or transitions started together drift apart.
* **`old` is the node's previous `ComputedStates`.** Pass `None` only when the
  node has never been styled — CSS does not transition an element's *initial*
  style, and the animator drops any leftover tracks in that case.
* **Skipping unchanged nodes is an optimisation, not a requirement.** Calling
  `on_computed` with `old == new` is explicitly safe: a running transition keeps
  running (see the `restyle_with_the_same_target_does_not_restart` test). The
  `PartialEq` guard exists only to avoid the per-node work.
* **`take_diagnostics()` drains one-shot advisories.** Each distinct message is
  emitted at most once for the animator's lifetime, so draining it every restyle
  is cheap and never spams.

Where the animator lives is the host's choice; the natural home is next to
`ENGINE` in `tree.rs`:

```rust
thread_local! {
    static ANIMATOR: RefCell<vn_css::Animator> = RefCell::new(vn_css::Animator::new());
}
```

### 1.2 In the render walk — `Animator::overlay`

```rust
pub fn overlay(&self, node: u32, now: Instant) -> Option<StyleRefinement>
```

The overlay is the set of *interpolated* values for this frame. It is refined
**on top of** the computed base, before the hover/active ladder is installed —
crates/vn-tree/src/render.rs, `build_node`:

```rust
if let Some(cs) = &node.computed {
    *d.style() = (*cs.base).clone();
    if let Some(overlay) = animator.overlay(id, frame_now) {
        d.style().refine(&overlay);          // gpui::Refineable
    }
    if let Some(h) = cs.hover.clone() { d = d.hover(move |_| (*h).clone()); }
    if let Some(a) = cs.active.clone() { d = d.active(move |_| (*a).clone()); }
}
```

* `StyleRefinement: Refineable<Refinement = StyleRefinement>`, so
  `refine(&overlay)` recurses correctly into the nested `text`, `padding`,
  `corner_radii`, … refinements and touches nothing the overlay leaves `None`.
* **`now` must be captured once per frame** (top of `build_root`), not per node,
  or elements animating together skew.
* `overlay` returns `None` for idle nodes — the common path costs one
  `HashMap` lookup.
* Ordering: the overlay animates the **base** state only. gpui evaluates
  `hover`/`active` on top of whatever base we hand it, so a hovered element
  still shows the hover refinement for the properties that refinement sets.

### 1.3 Keeping the frame pump alive — `any_active`

```rust
pub fn any_active(&self, now: Instant) -> bool
pub fn settles_at(&self) -> Option<Instant>   // last end instant, for timers
pub fn finish_settled(&mut self, now: Instant)
pub fn gc(&mut self, alive: &dyn Fn(u32) -> bool)
pub fn set_reduce_motion(&mut self, on: bool)
```

vn-app already re-arms `window.on_next_frame(pump_frame)` unconditionally, but
`on_next_frame` does not *repaint* — a repaint needs `cx.notify()` (or
`window.request_animation_frame()`). At the end of `pump_frame`:

```rust
if vn_tree::flush_changes() || vn_tree::animating() {
    shell.update(cx, |_, cx| cx.notify());
}
```

with, in vn-tree:

```rust
/// True while any transition is still running — the host keeps repainting.
pub fn animating() -> bool {
    ANIMATOR.with(|a| a.borrow().any_active(std::time::Instant::now()))
}
```

Housekeeping:

* **`finish_settled(now)`** drops tracks that reached their target. Safe to call
  right after painting: a finished track's value equals the computed base, so
  removing it is visually a no-op. Call it once per frame to keep the map small
  (or never — `any_active` already ignores finished tracks).
* **`gc(&|id| tree.node(id).is_some())`** belongs next to
  `render::gc_node_state()` in `flush_changes`.
* **`set_reduce_motion(cx.reduce_motion())`** mirrors gpui's own animations:
  while set, values snap and nothing is scheduled. Enabling it cancels whatever
  is running.

### 1.4 Inline `style="transition: …"`

`parse_inline` cannot carry transitions (it returns only a `StyleRefinement`), so
it now **advises** when it drops one. To honour inline transitions:

```rust
let (refinement, transitions) = vn_css::parse_inline_with_transitions(body, &mut diags);
// …then, in the cascade:
engine.compute_with(node_ref, Some(&refinement), Some(&Rc::new(transitions)))
```

`compute_with` is `compute` plus the inline transition list; inline is the last
cascade origin, so it replaces whatever the sheet declared. `compute` itself is
unchanged and simply forwards with `None`.

---

## 2. Parsing coverage

| Declaration | Status |
| --- | --- |
| `transition` (shorthand, comma list) | full — property / duration / delay / timing in any order, per the CSS grammar |
| `transition-property` | full, incl. `none`, `all`, shorthand names |
| `transition-duration` | full (`s`, `ms`) |
| `transition-delay` | parsed; **negative values clamped to 0** with an advisory |
| `transition-timing-function` | `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out`, `cubic-bezier()`, `steps()`, `step-start`, `step-end` |
| `transition: var(--x)` | consumed with an advisory (custom properties are not resolved in v1) |

Semantics implemented:

* The **shorthand resets all four longhands**; a longhand after it overrides
  just that component (both directions tested).
* `transition-property` decides the transition count; duration / delay /
  timing-function lists **cycle** to match, exactly as CSS does.
* A missing `transition-property` with any other transition longhand present
  means `all` (the CSS initial value), so `transition-duration: 200ms` alone
  works.
* Shorthand *names* inside `transition-property` expand to their longhands and
  each expanded longhand inherits that list entry's timing:
  `padding` → 4 sides, `margin` → 4 sides, `border-radius` → 4 corners,
  `gap` → `row-gap` + `column-gap`, `background` → `background-color`,
  `border` → `border-color`.
* Per property, the **last matching list entry wins** (`spec_for`); `all`
  matches everything, so `transition: all 1s, opacity 2s` gives opacity 2s and
  everything else 1s.
* `steps()` follows the CSS Easing Level 1 step-easing algorithm
  (`jump-start`/`start`, `jump-end`/`end`, `jump-none`, `jump-both`).
* `cubic-bezier()` is inverted with Newton-Raphson and a bisection fallback,
  checked against a dense brute-force inversion and against the published
  values for the keyword curves.

### Cascade folding

`transition` does **not** merge across rules: the last base-state rule that
declares any transition **replaces the whole list**, `!important` on top, inline
last. `transition: none` therefore clears it. `ComputedStates::transitions` is an
`Rc<Vec<TransitionSpec>>`; empty means "no transitions".

---

## 3. The animatable matrix

| Property | gpui slot | Interpolation | Notes |
| --- | --- | --- | --- |
| `opacity` | `opacity` | `f32` | used value clamped to 0..1 |
| `color` | `text.color` | `Hsla` (shortest hue arc) | |
| `background-color`, `background` | `background` | `Hsla` | **solid `Fill` only**; a gradient/pattern endpoint snaps |
| `border-color`, `border` | `border_color` | `Hsla` | gpui keeps one color for all four sides |
| `width`, `height` | `size.{width,height}` | `Pixels` / `Rems` | both ends must be absolute |
| `min-width`, `min-height` | `min_size.*` | `Pixels` / `Rems` | |
| `max-width`, `max-height` | `max_size.*` | `Pixels` / `Rems` | |
| `padding-{top,right,bottom,left}` | `padding.*` | `Pixels` / `Rems` | |
| `margin-{top,right,bottom,left}` | `margin.*` | `Pixels` / `Rems` | |
| `border-*-radius` (4 corners) | `corner_radii.*` | `Pixels` / `Rems` | |
| `row-gap`, `column-gap` | `gap.height`, `gap.width` | `Pixels` / `Rems` | gpui's `gap` is a `Size` |
| `font-size` | `text.font_size` | `Pixels` / `Rems` | |

Everything else named by a `transition` becomes `TransitionProperty::Other` and
gets the one-shot advisory **"transition of `<prop>` not animatable yet"**.
`all` expands to exactly the 25 rows above (`ANIMATABLE_COUNT`).

### Endpoints that cannot be interpolated

The value **snaps** (no track) and a one-shot advisory is emitted when:

| Case | Advisory |
| --- | --- |
| percentage ↔ px, `auto`/`none` on either end | `transition of <p> between percentage/auto endpoints is not animatable yet (v1) — snapping` |
| one end unset and the property has no resolvable initial (`color`, `border-color`, `font-size`, the sizes) | `transition of <p> from/to an unset value is not animatable yet (v1) — snapping` |
| px on one end, rem on the other (both non-zero) | `transition of <p> between different length units is not animatable yet (v1) — snapping` |
| a non-solid `Fill` on either end | as the percentage/auto case |

An **unchanged** non-animatable value is silent — `Slot` keeps enough of the
original shape (`Auto`, `Fraction(f)`, `NonSolidFill`) to compare, so a `width:
auto` element that never changes never warns.

Where CSS *does* define a usable initial value, an absent endpoint is filled in
rather than snapping: `opacity` → `1`, `background-color` → `transparent`,
padding / margin / gap / border-radius → `0px`. `0px` and `0rem` unify.

---

## 4. Timing model

```rust
pub struct TransitionSpec {
    pub property: TransitionProperty,
    pub duration: Duration,
    pub delay: Duration,
    pub easing: Easing,
}
```

* Input progress is `0` for the whole delay, then
  `(elapsed - delay) / duration` clamped to `0..=1`.
* Output progress is `easing.eval(input)` and **may leave `0..=1`** — a
  `cubic-bezier` with `y` outside the unit interval overshoots, which is correct
  CSS. Colors are clamped per channel and `opacity` is clamped to `0..=1` at
  install time (the used-value clamp browsers apply).
* A **zero combined duration** (`duration == 0 && delay == 0`) starts no
  transition, per spec. `duration: 0` with a positive delay *does* start one: the
  value jumps when the delay elapses, exactly as in a browser.
* **Retargeting preserves visual continuity.** When a property changes while its
  transition is mid-flight, the new track starts from the value being shown
  *right now* (the interpolated value), not from the old computed value. If the
  new target equals the running track's target, the track is left completely
  alone — no restart, no timing reset.
* Dropping a property out of `transition-property`, or removing the `transition`
  declaration entirely, **cancels** its running transition (the value snaps),
  matching CSS.

---

## 5. Known deviations from the browser (v1)

Each of these is surfaced as a diagnostic rather than being silent, per the
project's browser-compat-first policy.

1. **State-rule transitions are ignored.** `.a:hover { transition: … }` is
   rejected at sheet-compile time with a diagnostic telling the author to move
   it to the base rule. `:hover`/`:active` refinements are installed into gpui's
   own interactivity ladder and never pass through the animator, so a transition
   declared there cannot run. Transitions declared on the *base* rule still do
   not animate the hover ladder — v1 animates class/inline-driven base changes,
   which is what Vue apps overwhelmingly use.
2. **Transitions fold whole-list, not per-longhand.** A later rule that sets only
   `transition-duration` replaces the earlier rule's whole transition list rather
   than cascading that one longhand. Within a single declaration block the
   longhands compose correctly.
3. **Negative `transition-delay` is clamped to 0.** CSS starts such a transition
   already partway through; `std::time::Duration` cannot be negative.
4. **No reversing-shortening rule.** CSS shortens a transition that reverses
   before finishing so the return trip takes proportionally less time. v1 always
   uses the full duration from the interrupted value.
5. **`transitionrun` / `transitionstart` / `transitionend` events are not
   emitted.** Nothing is dispatched into JS.
6. **`transition-behavior: allow-discrete`, `@starting-style`, and discrete
   interpolation are unimplemented** — discrete properties simply snap.
7. **`em` is treated as `16px`** by the mapper (no parent font-size context yet),
   so `font-size` transitions in `em` interpolate against that constant.
8. **One border color.** gpui has a single `border_color`, so `border-top-color`
   and friends are neither mapped nor animatable; `border-color` transitions the
   one color.
9. **`calc()` with a percentage in it is laid out by the sizing properties
   only** (m26): width/height and their min/max, inset, margin, padding, gap and
   flex-basis take any LINEAR calc — sums and scalings, `calc(38.89% + 13.33px)`,
   `calc((100% - 1rem) * 0.5)` — as gpui's `DefiniteLength::Calc`, resolved at
   layout like a percentage. Everywhere else a percentage is resolved before
   layout, so a calc mixing one in (and `min()` / `max()` / `clamp()` over a
   percentage anywhere) drops the declaration, as if it were invalid, with the
   advisory "`<prop>`: this calc() is dropped (v1)". A calc of lengths alone is
   a constant and works everywhere. A transition with a calc endpoint snaps.
10. **`background-image: url()` is painted by vn-tree, not by gpui** (m26). The
    mapper carries each `url()` layer with its CSS index (`Extras::
    background_urls`) and `background-size` keeps `cover` / `contain`;
    `vn_tree::bgimage` loads the URL through `<img src>`'s resolver and paints
    the whole stack, gradients included and in CSS order, from a child laid
    over the element's padding box. Size, position, per-axis repeat and rounded
    corners follow CSS. What differs: the painting area is the padding box
    (`background-clip: padding-box`, so nothing under a translucent border); a
    relative URL resolves against the app root rather than the stylesheet; in
    a scroll container the image scrolls with the content (`background-
    attachment: local`); `space` / `round` repeat plainly; an animated GIF
    shows its first frame; an SVG with no width/height takes its viewBox size;
    `image-set()` is still not painted (advisory); and a layer needing more than
    4096 tiles is skipped (advisory). A URL that fails to load paints nothing,
    with an advisory.
11. **The user-agent sheet is one rule, `[hidden] { display: none }`** (m26).
    `Engine::compile` collects it ahead of the author's sheet as
    `:where([hidden])`. Specificity 0 and the earliest order put it below every
    author declaration, as the UA origin is, so `[hidden]` with an author
    `display: flex` shows the element, as it does in a browser. An author
    `!important` beats it too. That holds in a browser as well, because the UA
    rule is normal and not `!important`. `hidden="until-found"` is
    `display: none` here. A browser keeps that box (`content-visibility:
    hidden`) so find-in-page can reveal it. vue-native has no find-in-page,
    so the difference shows only in layout reads: the box measures zero here.
12. **`mask` is applied in one shape only, the gradient-border ring** (m26). That
    is two opaque layers, the top one clipped to `content-box`, composited with
    `mask-composite: exclude` (`-webkit-mask-composite: xor` or
    `destination-out`): `mask: linear-gradient(#fff 0 0) content-box,
    linear-gradient(#fff 0 0)`. The shorthand, `mask-image` + `mask-clip`, and
    either composite property all spell it, and each cascades on its own.
    gpui then paints the element's background only in its padding and border
    (`Style::vn_mask_ring`). What differs: the element's children are not
    masked (advisory); its outer and inset shadows, outline, backdrop blur and
    `--shading` are not painted; a percentage padding resolves against the
    element's own width; and a `url()` background is not masked (advisory).
    Every other mask (a fade, an icon `url()`) paints the element unmasked,
    with the advisory "`mask` is not applied (v1)".
13. **`border-radius` corners are circles** (m26). A corner of one length goes
    in gpui's `corner_radii`. A percentage, a linear `calc()` with one, or an
    elliptical `a / b` corner goes in the fork's `vn_corner_radii`, which gpui
    resolves against the box at paint: the horizontal radius against the
    width, the vertical against the height (`gpui::vn_resolve_corner_radii`).
    CSS's overlap rule scales radii whose sum overruns a side, which is what
    makes `rounded-full` (9999px) a pill. What differs:
    - gpui draws circular corners, so a corner whose two radii differ takes
      the smaller one. `50%` on a box that is not square is a pill where a
      browser draws an ellipse. vn-tree advises once when that happens.
    - Each radius is then clamped to half the box's shorter side, because the
      shaders pick a corner by quadrant. CSS lets a lone large corner reach
      past the middle.
    - A transition does not interpolate such a corner. Toward one it snaps;
      from one to a length it runs from 0.
    The same radii clip `overflow` (docs/ARCHITECTURE.md, fork items 33 and
    34).

---

## 6. Tests

`cargo test --target x86_64-pc-windows-msvc -p vn-css` — 79 unit tests:

* `transition::tests` (23) — shorthand/longhand parsing, list cycling,
  shorthand-name expansion, `none`, shorthand/longhand override order,
  `spec_for`/`covered`, negative delay and out-of-range bézier advisories,
  cubic-bézier against known values + a brute-force inversion, the `steps()`
  algorithm, endpoint exactness, monotonicity, overshoot.
* `animator::tests` (26) — diff → start → interpolate → settle, delay, zero
  duration, retarget continuity (both "resumes from the shown value" and "does
  not restart on an unchanged target"), Hsla shortest-arc color, px lengths,
  CSS-initial substitution, percentage/auto/unit snapping with one-shot
  advisories, `all` expansion, cancellation, gc/clear, `settles_at`,
  reduce-motion, opacity clamping, `refine`-on-top-of-base.
* `mapper::tests` (19) — the property arms and their advisories: overflow /
  gap / border-width / border-radius longhands, `border-style: none` computing
  the widths to zero, the `flex` shorthand, `flex-wrap`, `align-self` /
  `align-content`, `grid-template-columns: repeat(N, minmax(0, 1fr))` (and the
  advisory for track lists gpui cannot model), `font-style` / `white-space` /
  `text-decoration-line` / `text-overflow`, `visibility`, `aspect-ratio`,
  one-shot folding of `--*` declarations and of repeated unmapped properties,
  the `currentColor` advisory, and an end-to-end block of real Tailwind v3.4
  output.
* `tests` in `lib.rs` (11) — cascade folding: specificity, later-rule
  replacement, `transition: none`, `!important`, state-rule rejection, inline
  override via `compute_with`, no false "property not mapped" diagnostics,
  `ComputedStates` value equality (the change-detection primitive from §1.1),
  and the two Tailwind `transition-colors` / `duration-*` cases that pin how
  deviation 2 (whole-list folding) plays out for utility CSS.
