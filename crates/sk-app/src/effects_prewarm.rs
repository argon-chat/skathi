//! `"effects": { "prewarm" }` in sk.config.json: which of the app's `--shading` effects the renderers
//! build ahead of the frame that first draws them. Off by default.

use gpui::AppEffectsPrewarm;

fn configured(window_json: Option<&str>) -> AppEffectsPrewarm {
    let Some(config) = window_json.and_then(crate::json::parse) else {
        return AppEffectsPrewarm::Off;
    };
    let Some(prewarm) = config.get("effects").and_then(|e| e.get("prewarm")) else {
        return AppEffectsPrewarm::Off;
    };
    if let Some(on) = prewarm.as_bool() {
        return if on {
            AppEffectsPrewarm::All
        } else {
            AppEffectsPrewarm::Off
        };
    }
    let names = prewarm.as_arr().map(|names| {
        names
            .iter()
            .map(|name| {
                name.as_str()
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
            })
            .collect::<Option<Vec<_>>>()
    });
    match names.flatten() {
        Some(names) => AppEffectsPrewarm::Only(names),
        None => {
            log::warn!(
                "sk.config.json \"effects.prewarm\" must be true, false or a list of effect names; \
                 the app's effects are built in the first frame that draws them"
            );
            AppEffectsPrewarm::Off
        }
    }
}

/// At start, before the first window.
pub(crate) fn apply(window_json: Option<&str>) {
    gpui::set_app_effects_prewarm(configured(window_json));
}
