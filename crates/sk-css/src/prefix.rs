//! Vendor-prefixed properties. A prefixed declaration is applied only when its spelling is in
//! [`WEBKIT_APPLIED`]; every other vendor prefix is dropped before the mapper with no advisory.
//! Prefixed values belong to the property they are a value of.

pub const WEBKIT_APPLIED: &[&str] = &[
    "align-content",
    "align-items",
    "align-self",
    "animation",
    "animation-delay",
    "animation-direction",
    "animation-duration",
    "animation-fill-mode",
    "animation-iteration-count",
    "animation-name",
    "animation-play-state",
    "animation-timing-function",
    "app-region",
    "appearance",
    "backdrop-filter",
    "backface-visibility",
    "background-clip",
    "background-origin",
    "background-size",
    "border-bottom-left-radius",
    "border-bottom-right-radius",
    "border-radius",
    "border-top-left-radius",
    "border-top-right-radius",
    "box-align",
    "box-decoration-break",
    "box-direction",
    "box-flex",
    "box-ordinal-group",
    "box-orient",
    "box-pack",
    "box-reflect",
    "box-shadow",
    "box-sizing",
    "clip-path",
    "column-count",
    "column-gap",
    "column-rule",
    "column-span",
    "column-width",
    "columns",
    "filter",
    "flex",
    "flex-basis",
    "flex-direction",
    "flex-flow",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "font-feature-settings",
    "font-smoothing",
    "hyphens",
    "justify-content",
    "line-clamp",
    "mask",
    "mask-clip",
    "mask-composite",
    "mask-image",
    "mask-origin",
    "mask-position",
    "mask-repeat",
    "mask-size",
    "order",
    "perspective",
    "perspective-origin",
    "text-fill-color",
    "text-size-adjust",
    "text-stroke",
    "text-stroke-color",
    "text-stroke-width",
    "transform",
    "transform-origin",
    "transform-style",
    "transition",
    "transition-delay",
    "transition-duration",
    "transition-property",
    "transition-timing-function",
    "user-select",
    "writing-mode",
];

const PREFIXES: [&str; 4] = ["-webkit-", "-moz-", "-ms-", "-o-"];

pub fn dropped(name: &str) -> bool {
    let Some(prefix) = PREFIXES.iter().find(|p| starts_with_ignore_case(name, p)) else {
        return false;
    };
    if *prefix != "-webkit-" {
        return true;
    }
    let base = &name[prefix.len()..];
    !WEBKIT_APPLIED.iter().any(|n| n.eq_ignore_ascii_case(base))
}

/// As `dropped`, for a parsed property: lightningcss strips the prefix from a property it knows and
/// keeps the whole name of one it does not.
pub(crate) fn property_dropped(p: &lightningcss::properties::Property) -> bool {
    use lightningcss::properties::custom::CustomPropertyName;
    use lightningcss::properties::PropertyId;
    use lightningcss::vendor_prefix::VendorPrefix;
    let id = p.property_id();
    match &id {
        PropertyId::Custom(CustomPropertyName::Unknown(name)) => dropped(name.0.as_ref()),
        PropertyId::Custom(_) => false,
        _ => {
            let vp = id.prefix();
            let prefix = if vp.contains(VendorPrefix::WebKit) {
                "-webkit-"
            } else if vp.contains(VendorPrefix::Moz) {
                "-moz-"
            } else if vp.contains(VendorPrefix::Ms) {
                "-ms-"
            } else if vp.contains(VendorPrefix::O) {
                "-o-"
            } else {
                return false;
            };
            if vp.contains(VendorPrefix::None) {
                return false;
            }
            dropped(&format!("{prefix}{}", id.name()))
        }
    }
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}
