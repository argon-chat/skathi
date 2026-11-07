//! Same-document SVG color filters over a rendered subtree.

use crate::tree::{Kind, Tree};
use gpui::VnColorMatrix;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

thread_local! {
    static FILTERS: RefCell<HashMap<String, u32>> = RefCell::new(HashMap::new());
    static WARNED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

pub(crate) fn resolve(tree: &Tree, url: &str, target: u32) -> Option<VnColorMatrix> {
    let result = resolve_inner(tree, url);
    match result {
        Ok(mut matrix) => { matrix.group_id = target as u64; Some(matrix) },
        Err(message) => {
            if WARNED.with(|warned| warned.borrow_mut().insert(format!("{url}:{message}"))) {
                crate::log::record("css", format!("[css] filter: url({url}): {message}"));
            }
            None
        }
    }
}

fn find_filter(tree: &Tree, name: &str) -> Option<u32> {
    if let Some(id) = FILTERS.with(|filters| filters.borrow().get(name).copied()) {
        if tree.prop(id, "id") == Some(name) && connected(tree, id) {
            return Some(id);
        }
    }
    // Definitions are usually shallow siblings of the app container. Breadth
    // first avoids walking a large chat history before reaching them.
    let mut pending = VecDeque::from([tree.root]);
    while let Some(id) = pending.pop_front() {
        let Some(node) = tree.node(id) else { continue };
        if tree.prop(id, "id") == Some(name) {
            FILTERS.with(|filters| filters.borrow_mut().insert(name.to_owned(), id));
            return Some(id);
        }
        pending.extend(node.children.iter().copied());
    }
    None
}

fn connected(tree: &Tree, mut id: u32) -> bool {
    for _ in 0..4096 {
        if id == tree.root && id != 0 {
            return true;
        }
        let Some(node) = tree.node(id) else {
            return false;
        };
        id = node.parent;
    }
    false
}

fn resolve_inner(tree: &Tree, url: &str) -> Result<VnColorMatrix, &'static str> {
    let name = url
        .strip_prefix('#')
        .filter(|name| !name.is_empty())
        .ok_or("only same-document filters are supported")?;
    let id = find_filter(tree, name).ok_or("SVG filter element was not found")?;
    let node = tree.node(id).ok_or("SVG filter element was removed")?;
    if !tree.name(node.tag).eq_ignore_ascii_case("filter") {
        return Err("reference is not an SVG filter");
    }
    let primitives: Vec<_> = node
        .children
        .iter()
        .copied()
        .filter(|child| {
            tree.node(*child)
                .is_some_and(|node| node.kind == Kind::Element)
        })
        .collect();
    if primitives.len() != 1 {
        return Err("expected one feColorMatrix primitive");
    }
    let primitive = primitives[0];
    if !tree
        .node(primitive)
        .is_some_and(|node| tree.name(node.tag).eq_ignore_ascii_case("feColorMatrix"))
    {
        return Err("only feColorMatrix is supported");
    }
    if tree
        .prop(primitive, "type")
        .is_some_and(|kind| kind != "matrix")
    {
        return Err("feColorMatrix type must be matrix");
    }
    if tree
        .prop(primitive, "in")
        .is_some_and(|input| input != "SourceGraphic")
    {
        return Err("feColorMatrix input must be SourceGraphic");
    }
    let values = match tree.prop(primitive, "values") {
        None => VnColorMatrix::IDENTITY,
        Some(text) => parse_matrix(text).ok_or("feColorMatrix needs 20 finite coefficients")?,
    };
    let mut cursor = primitive;
    let mut linear_rgb = true;
    for _ in 0..4096 {
        let Some(node) = tree.node(cursor) else { break };
        if let Some(value) = tree.prop(cursor, "color-interpolation-filters") {
            linear_rgb = value != "sRGB";
            break;
        }
        cursor = node.parent;
    }
    Ok(VnColorMatrix { group_id: 0, values, linear_rgb })
}

fn parse_matrix(text: &str) -> Option<[f32; 20]> {
    let values: Vec<f32> = text
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let values: [f32; 20] = values.try_into().ok()?;
    values
        .iter()
        .all(|value| value.is_finite())
        .then_some(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_accept_svg_number_lists_and_reject_partial_or_nonfinite_values() {
        let matrix =
            parse_matrix("0.567, .433, 0, 0, 0\n .558 .442 0 0 0 0 .242 .758 0 0 0 0 0 1 0")
                .expect("matrix");
        assert_eq!(matrix[0], 0.567);
        assert_eq!(matrix[18], 1.);
        assert!(parse_matrix("1 0 0 0").is_none());
        assert!(parse_matrix("NaN 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 1 0").is_none());
    }

    #[test]
    fn definition_updates_and_removal_are_observed_without_changing_filter_url() {
        crate::tree::with_tree(|tree| {
            let root = tree.create_root();
            let filter_tag = tree.intern("filter");
            let filter = tree.create_element(filter_tag);
            tree.insert(filter, root, 0);
            let id_key = tree.intern("id");
            tree.set_prop(filter, id_key, Some("test-color-filter".into()));
            let primitive_tag = tree.intern("feColorMatrix");
            let primitive = tree.create_element(primitive_tag);
            tree.insert(primitive, filter, 0);
            assert_eq!(
                resolve_inner(tree, "#test-color-filter")
                    .expect("identity")
                    .values,
                VnColorMatrix::IDENTITY
            );
            let values_key = tree.intern("values");
            tree.set_prop(
                primitive,
                values_key,
                Some("0 1 0 0 0 1 0 0 0 0 0 0 1 0 0 0 0 0 1 0".into()),
            );
            assert_eq!(
                resolve_inner(tree, "#test-color-filter")
                    .expect("updated")
                    .values[0],
                0.
            );
            tree.remove(filter);
            assert!(resolve_inner(tree, "#test-color-filter").is_err());
            tree.remove(root);
        });
    }
}
