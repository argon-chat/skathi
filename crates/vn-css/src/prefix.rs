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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_lowercase_unprefixed_and_unique() {
        for pair in WEBKIT_APPLIED.windows(2) {
            assert!(pair[0] < pair[1], "{} then {}", pair[0], pair[1]);
        }
        for n in WEBKIT_APPLIED {
            assert_eq!(*n, n.to_ascii_lowercase());
            assert!(!n.starts_with('-'), "{n}");
        }
    }

    #[test]
    fn only_moz_ms_o_and_unlisted_webkit_spellings_are_dropped() {
        for name in [
            "-moz-osx-font-smoothing",
            "-moz-tab-size",
            "-moz-user-select",
            "-ms-overflow-style",
            "-ms-flex",
            "-o-transition",
            "-webkit-touch-callout",
            "-webkit-font-smoothing",
            "-webkit-tap-highlight-color",
            "-webkit-overflow-scrolling",
            "-WEBKIT-TOUCH-CALLOUT",
        ] {
            assert!(dropped(name), "{name}");
        }
        for name in [
            "-webkit-app-region",
            "-webkit-line-clamp",
            "-webkit-box-orient",
            "-webkit-text-fill-color",
            "-webkit-background-clip",
            "-webkit-mask-image",
            "-webkit-mask-composite",
            "-webkit-user-select",
            "-webkit-backdrop-filter",
            "-WebKit-Transform",
            "transform",
            "--webkit-like",
            "touch-callout",
        ] {
            assert!(!dropped(name), "{name}");
        }
    }
}
