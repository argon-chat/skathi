//! Low-level access to the layout algorithms themselves. For a higher-level API, see the [`TaffyTree`](crate::TaffyTree) struct.
//!
//! ### Layout functions
//!
//! The layout functions all take an [`&mut impl LayoutPartialTree`](crate::LayoutPartialTree) parameter, which represents a single container node and it's direct children.
//!
//! | Function                          | Purpose                                                                                                                                                                                            |
//! | ---                               | ---                                                                                                                                                                                                |
//! | [`compute_flexbox_layout`]        | Layout a Flexbox container and it's direct children                                                                                                                                                |
//! | [`compute_grid_layout`]           | Layout a CSS Grid container and it's direct children                                                                                                                                               |
//! | [`compute_block_layout`]          | Layout a Block container and it's direct children                                                                                                                                                  |
//! | [`compute_leaf_layout`]           | Applies common properties like padding/border/aspect-ratio to a node before deferring to a passed closure to determine it's size. Can be applied to nodes like text or image nodes.                |
//! | [`compute_root_layout`]           | Layout the root node of a tree (regardless of it's layout mode). This function is typically called once to begin a layout run.                                                                     |                                                                      |
//! | [`compute_hidden_layout`]         | Mark a node as hidden during layout (like `Display::None`)                                                                                                                                         |
//! | [`compute_cached_layout`]         | Attempts to find a cached layout for the specified node and layout inputs. Uses the provided closure to compute the layout (and then stores the result in the cache) if no cached layout is found. |
//!
//! ### Other functions
//!
//! | Function                          | Requires                                                                                                                                                                                           | Purpose                                                              |
//! | ---                               | ---                                                                                                                                                                                                | ---                                                                  |
//! | [`round_layout`]                  | [`RoundTree`]                                                                                                                                                                                      | Round a tree of float-valued layouts to integer pixels               |
//! | [`print_tree`](crate::print_tree) | [`PrintTree`](crate::PrintTree)                                                                                                                                                                    | Print a debug representation of a node tree and it's computed layout |
//!
pub(crate) mod common;
pub(crate) mod leaf;

#[cfg(feature = "block_layout")]
pub(crate) mod block;

#[cfg(feature = "float_layout")]
pub(crate) mod float;

#[cfg(feature = "flexbox")]
pub(crate) mod flexbox;

#[cfg(feature = "grid")]
pub(crate) mod grid;

pub use leaf::compute_leaf_layout;

#[cfg(feature = "block_layout")]
pub use self::block::{compute_block_layout, BlockContext, BlockFormattingContext};

#[cfg(feature = "flexbox")]
pub use self::flexbox::compute_flexbox_layout;

#[cfg(feature = "grid")]
pub use self::grid::compute_grid_layout;

#[cfg(feature = "float_layout")]
pub use self::float::{BfcSlot, ContentSlot, FloatContext, FloatIntrinsicWidthCalculator};

use crate::geometry::{Line, Point, Size};
use crate::style::{AvailableSpace, CoreStyle, Overflow};
use crate::tree::{
    Layout, LayoutInput, LayoutOutput, LayoutPartialTree, LayoutPartialTreeExt, NodeId, RoundTree, SizingMode,
};
use crate::util::debug::{debug_log, debug_log_node, debug_pop_node, debug_push_node};
use crate::util::sys::round;
use crate::util::ResolveOrZero;
use crate::{CacheTree, MaybeMath, MaybeResolve};

/// Compute layout for the root node in the tree
pub fn compute_root_layout(tree: &mut impl LayoutPartialTree, root: NodeId, available_space: Size<AvailableSpace>) {
    let mut known_dimensions = Size::NONE;

    #[cfg(feature = "block_layout")]
    {
        use crate::BoxSizing;

        let parent_size = available_space.into_options();
        let style = tree.get_core_container_style(root);

        if style.is_block() {
            // Pull these out earlier to avoid borrowing issues
            let aspect_ratio = style.aspect_ratio();
            let margin = style.margin().resolve_or_zero(parent_size.width, |val, basis| tree.calc(val, basis));
            let padding = style.padding().resolve_or_zero(parent_size.width, |val, basis| tree.calc(val, basis));
            let border = style.border().resolve_or_zero(parent_size.width, |val, basis| tree.calc(val, basis));
            let padding_border_size = (padding + border).sum_axes();
            let box_sizing_adjustment =
                if style.box_sizing() == BoxSizing::ContentBox { padding_border_size } else { Size::ZERO };

            let min_size = style
                .min_size()
                .maybe_resolve(parent_size, |val, basis| tree.calc(val, basis))
                .maybe_apply_aspect_ratio(aspect_ratio)
                .maybe_add(box_sizing_adjustment);
            let max_size = style
                .max_size()
                .maybe_resolve(parent_size, |val, basis| tree.calc(val, basis))
                .maybe_apply_aspect_ratio(aspect_ratio)
                .maybe_add(box_sizing_adjustment);
            let clamped_style_size = style
                .size()
                .maybe_resolve(parent_size, |val, basis| tree.calc(val, basis))
                .maybe_apply_aspect_ratio(aspect_ratio)
                .maybe_add(box_sizing_adjustment)
                .maybe_clamp(min_size, max_size);

            // If both min and max in a given axis are set and max <= min then this determines the size in that axis
            let min_max_definite_size = min_size.zip_map(max_size, |min, max| match (min, max) {
                (Some(min), Some(max)) if max <= min => Some(min),
                _ => None,
            });

            // Block nodes automatically stretch fit their width to fit available space if available space is definite
            let available_space_based_size = Size {
                width: available_space.width.into_option().maybe_sub(margin.horizontal_axis_sum()),
                height: None,
            };

            let styled_based_known_dimensions = known_dimensions
                .or(min_max_definite_size)
                .or(clamped_style_size)
                .or(available_space_based_size)
                .maybe_max(padding_border_size);

            known_dimensions = styled_based_known_dimensions;
        }
    }

    // Recursively compute node layout
    let output = tree.perform_child_layout(
        root,
        known_dimensions,
        available_space.into_options(),
        available_space,
        SizingMode::InherentSize,
        Line::FALSE,
    );
    let style = tree.get_core_container_style(root);
    let padding =
        style.padding().resolve_or_zero(available_space.width.into_option(), |val, basis| tree.calc(val, basis));
    let border =
        style.border().resolve_or_zero(available_space.width.into_option(), |val, basis| tree.calc(val, basis));
    let margin =
        style.margin().resolve_or_zero(available_space.width.into_option(), |val, basis| tree.calc(val, basis));
    let scrollbar_size = Size {
        width: if style.overflow().y == Overflow::Scroll { style.scrollbar_width() } else { 0.0 },
        height: if style.overflow().x == Overflow::Scroll { style.scrollbar_width() } else { 0.0 },
    };
    let location = Point {
        x: if style.direction().is_rtl() {
            available_space.width.into_option().map_or(0.0, |available_width| available_width - output.size.width)
        } else {
            0.0
        },
        y: 0.0,
    };
    drop(style);

    tree.set_unrounded_layout(
        root,
        &Layout {
            order: 0,
            location,
            size: output.size,
            #[cfg(feature = "content_size")]
            content_size: output.content_size,
            scrollbar_size,
            padding,
            border,
            // TODO: support auto margins for root node?
            margin,
        },
    );
}

/// Attempts to find a cached layout for the specified node and layout inputs.
///
/// Uses the provided closure to compute the layout (and then stores the result in the cache) if no cached layout is found.
#[inline(always)]
pub fn compute_cached_layout<Tree: CacheTree + ?Sized, ComputeFunction>(
    tree: &mut Tree,
    node: NodeId,
    inputs: LayoutInput,
    compute_uncached: ComputeFunction,
) -> LayoutOutput
where
    ComputeFunction: FnOnce(&mut Tree, NodeId, LayoutInput) -> LayoutOutput,
{
    debug_push_node!(node);

    // First we check if we have a cached result for the given input
    let cache_entry = tree.cache_get(node, &inputs);
    if let Some(cached_size_and_baselines) = cache_entry {
        debug_log_node!(inputs);
        debug_log!("RESULT (CACHED)", dbg:cached_size_and_baselines.size);
        debug_pop_node!();
        return cached_size_and_baselines;
    }

    debug_log_node!(inputs);

    let computed_size_and_baselines = compute_uncached(tree, node, inputs);

    // Cache result
    tree.cache_store(node, &inputs, computed_size_and_baselines);

    debug_log!("RESULT", dbg:computed_size_and_baselines.size);
    debug_pop_node!();

    computed_size_and_baselines
}

/// Rounds the calculated layout to exact pixel values
///
/// In order to ensure that no gaps in the layout are introduced we:
///   - Always round based on the cumulative x/y coordinates (relative to the viewport) rather than
///     parent-relative coordinates
///   - Compute width/height by first rounding the top/bottom/left/right and then computing the difference
///     rather than rounding the width/height directly
///
/// See <https://github.com/facebook/yoga/commit/aa5b296ac78f7a22e1aeaf4891243c6bb76488e2> for more context
///
/// In order to prevent innacuracies caused by rounding already-rounded values, we read from `unrounded_layout`
/// and write to `final_layout`.
pub fn round_layout(tree: &mut impl RoundTree, node_id: NodeId) {
    return round_layout_inner(tree, node_id, 0.0, 0.0);

    /// Recursive function to apply rounding to all descendents
    fn round_layout_inner(tree: &mut impl RoundTree, node_id: NodeId, cumulative_x: f32, cumulative_y: f32) {
        let unrounded_layout = tree.get_unrounded_layout(node_id);
        let mut layout = unrounded_layout;

        let cumulative_x = cumulative_x + unrounded_layout.location.x;
        let cumulative_y = cumulative_y + unrounded_layout.location.y;

        layout.location.x = round(unrounded_layout.location.x);
        layout.location.y = round(unrounded_layout.location.y);
        layout.size.width = round(cumulative_x + unrounded_layout.size.width) - round(cumulative_x);
        layout.size.height = round(cumulative_y + unrounded_layout.size.height) - round(cumulative_y);
        layout.scrollbar_size.width = round(unrounded_layout.scrollbar_size.width);
        layout.scrollbar_size.height = round(unrounded_layout.scrollbar_size.height);
        layout.border.left = round(cumulative_x + unrounded_layout.border.left) - round(cumulative_x);
        layout.border.right = round(cumulative_x + unrounded_layout.size.width)
            - round(cumulative_x + unrounded_layout.size.width - unrounded_layout.border.right);
        layout.border.top = round(cumulative_y + unrounded_layout.border.top) - round(cumulative_y);
        layout.border.bottom = round(cumulative_y + unrounded_layout.size.height)
            - round(cumulative_y + unrounded_layout.size.height - unrounded_layout.border.bottom);
        layout.padding.left = round(cumulative_x + unrounded_layout.padding.left) - round(cumulative_x);
        layout.padding.right = round(cumulative_x + unrounded_layout.size.width)
            - round(cumulative_x + unrounded_layout.size.width - unrounded_layout.padding.right);
        layout.padding.top = round(cumulative_y + unrounded_layout.padding.top) - round(cumulative_y);
        layout.padding.bottom = round(cumulative_y + unrounded_layout.size.height)
            - round(cumulative_y + unrounded_layout.size.height - unrounded_layout.padding.bottom);

        #[cfg(feature = "content_size")]
        round_content_size(&mut layout, unrounded_layout.content_size, cumulative_x, cumulative_y);

        tree.set_final_layout(node_id, &layout);

        let child_count = tree.child_count(node_id);
        for index in 0..child_count {
            let child = tree.get_child_id(node_id, index);
            round_layout_inner(tree, child, cumulative_x, cumulative_y);
        }
    }

    #[cfg(feature = "content_size")]
    #[inline(always)]
    /// Round content size variables.
    /// This is split into a separate function to make it easier to feature flag.
    fn round_content_size(
        layout: &mut Layout,
        unrounded_content_size: Size<f32>,
        cumulative_x: f32,
        cumulative_y: f32,
    ) {
        layout.content_size.width = round(cumulative_x + unrounded_content_size.width) - round(cumulative_x);
        layout.content_size.height = round(cumulative_y + unrounded_content_size.height) - round(cumulative_y);
    }
}

/// Creates a layout for this node and its children, recursively.
/// Each hidden node has zero size and is placed at the origin
pub fn compute_hidden_layout(tree: &mut (impl LayoutPartialTree + CacheTree), node: NodeId) -> LayoutOutput {
    // Clear cache and set zeroed-out layout for the node
    tree.cache_clear(node);
    tree.set_unrounded_layout(node, &Layout::with_order(0));

    // Perform hidden layout on all children
    for index in 0..tree.child_count(node) {
        let child_id = tree.get_child_id(node, index);
        tree.compute_child_layout(child_id, LayoutInput::HIDDEN);
    }

    LayoutOutput::HIDDEN
}

/// A module for unified re-exports of detailed layout info structs, used by low level API
#[cfg(feature = "detailed_layout_info")]
pub mod detailed_info {
    #[cfg(feature = "grid")]
    pub use super::grid::{DetailedGridInfo, DetailedGridItemsInfo, DetailedGridTracksInfo};
}

#[cfg(test)]
mod tests {
    use super::compute_hidden_layout;
    use crate::geometry::{Point, Size};
    use crate::style::{Display, Style};
    use crate::TaffyTree;

    #[test]
    fn hidden_layout_should_hide_recursively() {
        let mut taffy: TaffyTree<()> = TaffyTree::new();

        let style: Style = Style { display: Display::Flex, size: Size::from_lengths(50.0, 50.0), ..Default::default() };

        let grandchild_00 = taffy.new_leaf(style.clone()).unwrap();
        let grandchild_01 = taffy.new_leaf(style.clone()).unwrap();
        let child_00 = taffy.new_with_children(style.clone(), &[grandchild_00, grandchild_01]).unwrap();

        let grandchild_02 = taffy.new_leaf(style.clone()).unwrap();
        let child_01 = taffy.new_with_children(style.clone(), &[grandchild_02]).unwrap();

        let root = taffy
            .new_with_children(
                Style { display: Display::None, size: Size::from_lengths(50.0, 50.0), ..Default::default() },
                &[child_00, child_01],
            )
            .unwrap();

        compute_hidden_layout(&mut taffy.as_layout_tree(), root);

        // Whatever size and display-mode the nodes had previously,
        // all layouts should resolve to ZERO due to the root's DISPLAY::NONE

        for node in [root, child_00, child_01, grandchild_00, grandchild_01, grandchild_02] {
            let layout = taffy.layout(node).unwrap();
            assert_eq!(layout.size, Size::zero());
            assert_eq!(layout.location, Point::zero());
        }
    }
}

/// vue-native patch tests (vendor/taffy, m26): `Size::maybe_transfer_aspect_ratio`.
/// A picture (a 16:9 leaf) in a 100x120 box, under Tailwind's preflight
/// `max-width: 100%`.
#[cfg(all(test, feature = "taffy_tree", feature = "block_layout", feature = "flexbox", feature = "grid"))]
mod vn_aspect_transfer_tests {
    use crate::prelude::*;

    const RATIO: f32 = 16.0 / 9.0;

    /// The picture's laid-out size, as `(width, height)`.
    fn picture(parent: Display, absolute: bool, size: Size<Dimension>) -> (f32, f32) {
        let mut t: TaffyTree<()> = TaffyTree::new();
        t.disable_rounding();
        let pic = t
            .new_leaf(Style {
                position: if absolute { Position::Absolute } else { Position::Relative },
                inset: if absolute {
                    Rect { left: length(0.0), right: length(0.0), top: length(0.0), bottom: length(0.0) }
                } else {
                    Rect::auto()
                },
                size,
                max_size: Size { width: percent(1.0), height: auto() },
                aspect_ratio: Some(RATIO),
                ..Default::default()
            })
            .unwrap();
        let cell = t
            .new_with_children(
                Style {
                    display: parent,
                    size: Size { width: length(100.0), height: length(120.0) },
                    ..Default::default()
                },
                &[pic],
            )
            .unwrap();
        t.compute_layout(cell, Size::MAX_CONTENT).unwrap();
        let l = t.layout(pic).unwrap();
        (l.size.width, l.size.height)
    }

    fn full() -> Size<Dimension> {
        Size { width: percent(1.0), height: percent(1.0) }
    }

    #[test]
    fn a_definite_height_is_not_capped_by_max_width_over_the_ratio() {
        // VideoAttachment's poster: absolute, inset 0, 100% x 100%.
        for parent in [Display::Block, Display::Flex, Display::Grid] {
            assert_eq!(picture(parent, true, full()), (100.0, 120.0), "absolute in {parent:?}");
        }
        // `w-full h-full object-cover`, in flow. (Not in an `auto` grid track:
        // a percentage size there is cyclic, which is track sizing's question,
        // not this one.)
        for parent in [Display::Block, Display::Flex] {
            assert_eq!(picture(parent, false, full()), (100.0, 120.0), "in flow in {parent:?}");
        }
    }

    #[test]
    fn an_auto_height_still_follows_the_ratio_and_its_transferred_max() {
        // `width: 100%; height: auto`: the ratio decides the height.
        let auto_h = Size { width: percent(1.0), height: auto() };
        assert_eq!(picture(Display::Block, true, auto_h), (100.0, 100.0 / RATIO));
        assert_eq!(picture(Display::Block, false, auto_h), (100.0, 100.0 / RATIO));
        // `width: 500px; max-width: 100%; height: auto`: the width is capped at
        // 100 and the height follows it, through the transferred max-height.
        let wide = Size { width: length(500.0), height: auto() };
        assert_eq!(picture(Display::Block, true, wide), (100.0, 100.0 / RATIO));
        assert_eq!(picture(Display::Block, false, wide), (100.0, 100.0 / RATIO));
    }
}

/// vue-native patch tests (vendor/taffy, m26): a percentage `flex-basis` the
/// container cannot resolve is `content` (`determine_flex_base_size`).
#[cfg(all(test, feature = "taffy_tree", feature = "block_layout", feature = "flexbox"))]
mod vn_percent_basis_tests {
    use crate::prelude::*;

    /// The item's main size: `flex: 1 1 <basis>` with `<main>` as its own
    /// height (width for a row), holding content `content` px long, in a
    /// container whose main size is `container` (None: auto).
    fn item(column: bool, container: Option<f32>, basis: Dimension, main: Dimension, content: f32) -> f32 {
        let mut t: TaffyTree<()> = TaffyTree::new();
        let along = |m: Dimension, other: Dimension| if column { Size { width: other, height: m } } else { Size { width: m, height: other } };
        let inner = t.new_leaf(Style { size: along(length(content), length(10.0)), ..Default::default() }).unwrap();
        let it = t
            .new_with_children(
                Style {
                    display: Display::Flex,
                    flex_direction: if column { FlexDirection::Column } else { FlexDirection::Row },
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    flex_basis: basis,
                    size: along(main, auto()),
                    ..Default::default()
                },
                &[inner],
            )
            .unwrap();
        let fixed = |v: Option<f32>| v.map_or(auto(), length);
        let c = t
            .new_with_children(
                Style {
                    display: Display::Flex,
                    flex_direction: if column { FlexDirection::Column } else { FlexDirection::Row },
                    size: along(fixed(container), auto()),
                    ..Default::default()
                },
                &[it],
            )
            .unwrap();
        t.compute_layout(c, Size::MAX_CONTENT).unwrap();
        let s = t.layout(it).unwrap().size;
        if column {
            s.height
        } else {
            s.width
        }
    }

    #[test]
    fn a_definite_container_resolves_the_percentage_and_the_size_caps_the_minimum() {
        // Argon's settings drawer: `flex-1` (`flex: 1 1 0%`) and `height:
        // 100%` in a 700px column, over 3000px of settings. The automatic
        // minimum is the smaller of the content and the specified size, so the
        // row is held to the column and its scroller scrolls.
        for column in [true, false] {
            assert_eq!(item(column, Some(700.0), percent(0.0), percent(1.0), 3000.0), 700.0, "column {column}");
            assert_eq!(item(column, Some(700.0), percent(0.0), length(600.0), 3000.0), 700.0, "column {column}");
            // The same with a length basis, which was always right.
            assert_eq!(item(column, Some(700.0), length(0.0), percent(1.0), 3000.0), 700.0, "column {column}");
            // No specified size: the minimum is the content, in a browser too.
            assert_eq!(item(column, Some(700.0), percent(0.0), auto(), 3000.0), 3000.0, "column {column}");
        }
    }

    #[test]
    fn an_indefinite_container_makes_the_percentage_content_not_the_size() {
        // reka's Select viewport: `flex: 1` and `height: 40px` (the trigger's)
        // in an auto-height column, around 103px of items. Edge: 103.
        for column in [true, false] {
            assert_eq!(item(column, None, percent(0.0), length(40.0), 103.0), 103.0, "column {column}");
            // An `auto` basis still reads the size.
            assert_eq!(item(column, None, auto(), length(40.0), 103.0), 40.0, "column {column}");
        }
    }
}
