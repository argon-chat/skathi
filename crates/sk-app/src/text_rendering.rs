//! `"textRendering"` in sk.config.json: which rasterizer draws text.

use gpui::TextRendering;

fn parse(name: &str) -> Option<TextRendering> {
    match name {
        "skathi-mode" => Some(TextRendering::SkathiMode),
        "optimized" => Some(TextRendering::Optimized),
        _ => None,
    }
}

/// `SK_TEXT_RENDERING` decides over the embedder, the embedder over the app.
fn choose(
    from_env: Option<&str>,
    pinned: Option<TextRendering>,
    configured: Option<&str>,
) -> TextRendering {
    from_env
        .and_then(parse)
        .or(pinned)
        .or_else(|| configured.and_then(parse))
        .unwrap_or_else(TextRendering::platform_default)
}

/// Before the first window, so no glyph is rasterized in the other mode.
pub(crate) fn apply() {
    let from_env = std::env::var("SK_TEXT_RENDERING").ok();
    if let Some(value) = from_env.as_deref().filter(|v| parse(v).is_none()) {
        log::warn!(
            "SK_TEXT_RENDERING={value:?} is not a text rendering mode: use skathi-mode or optimized"
        );
    }
    let configured = crate::boot_mode::text_rendering();
    gpui::set_text_rendering(choose(
        from_env.as_deref(),
        crate::embed::pinned_text_rendering(),
        configured.as_deref(),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_decides_over_the_embedder_and_the_app() {
        use TextRendering::*;
        assert_eq!(
            choose(Some("optimized"), Some(SkathiMode), Some("skathi-mode")),
            Optimized
        );
        assert_eq!(
            choose(None, Some(Optimized), Some("skathi-mode")),
            Optimized
        );
        assert_eq!(choose(Some("bogus"), None, Some("optimized")), Optimized);
        assert_eq!(choose(None, None, None), TextRendering::platform_default());
    }
}
