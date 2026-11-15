//! The images `--shading` effects sample (`grade(url(assets/lut.png))`): resolved and loaded like
//! a `background-image: url()`, relative to the app root.

use std::{cell::RefCell, collections::HashMap, sync::Arc};

use gpui::{App, ImageSource, RenderImage, Window};

use crate::bgimage::{load, Load};

thread_local! {
    static SOURCES: RefCell<HashMap<u32, Option<ImageSource>>> = RefCell::new(HashMap::new());
}

pub(crate) fn install() {
    gpui::set_effect_texture_resolver(resolve);
}

fn resolve(source: u32, window: &mut Window, cx: &mut App) -> Option<Arc<RenderImage>> {
    let image = SOURCES.with(|sources| {
        sources
            .borrow_mut()
            .entry(source)
            .or_insert_with(|| {
                let image = crate::img::resolve(&sk_effects::texture_url(source)?);
                crate::img::note_known(image.as_ref());
                image
            })
            .clone()
    })?;
    match load(&image, window, cx) {
        Load::Ready(image) => Some(image),
        Load::Pending => None,
        Load::Failed => {
            let url = sk_effects::texture_url(source).unwrap_or_default();
            crate::render::warn_compat(
                &format!("effect-texture-failed:{url}"),
                &format!(
                    "--shading: url({url}) could not be loaded — the effect reads its fallback \
                     colour for it. The URL takes the same forms and is resolved the same way as \
                     an `<img src>`'s"
                ),
            );
            None
        }
    }
}
