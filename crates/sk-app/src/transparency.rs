//! `prefers-reduced-transparency`: Windows' "Transparency effects" switch (`UISettings`), watched so
//! a change re-evaluates `@media` without other input. Other platforms answer `no-preference`.

#[cfg(target_os = "windows")]
mod imp {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicU8, Ordering};

    use windows::Foundation::TypedEventHandler;
    use windows::UI::ViewManagement::UISettings;

    const UNREAD: u8 = 0;
    const OFF: u8 = 1;
    const REDUCED: u8 = 2;

    static STATE: AtomicU8 = AtomicU8::new(UNREAD);

    thread_local! {
        /// Kept alive for its change event.
        static SETTINGS: RefCell<Option<UISettings>> = const { RefCell::new(None) };
    }

    fn store(settings: &UISettings) -> bool {
        let reduced = settings.AdvancedEffectsEnabled().is_ok_and(|on| !on);
        STATE.store(if reduced { REDUCED } else { OFF }, Ordering::Relaxed);
        reduced
    }

    pub fn reduce_transparency() -> bool {
        match STATE.load(Ordering::Relaxed) {
            UNREAD => watch(),
            state => state == REDUCED,
        }
    }

    fn watch() -> bool {
        let Ok(settings) = UISettings::new() else {
            STATE.store(OFF, Ordering::Relaxed);
            return false;
        };
        let reduced = store(&settings);
        let handler = TypedEventHandler::new(|sender: windows::core::Ref<UISettings>, _| {
            if let Some(settings) = sender.as_ref() {
                store(settings);
                sk_runtime::frames::wake();
            }
            Ok(())
        });
        if settings.AdvancedEffectsEnabledChanged(&handler).is_ok() {
            SETTINGS.with(|s| *s.borrow_mut() = Some(settings));
        }
        reduced
    }
}

#[cfg(target_os = "windows")]
pub use imp::reduce_transparency;

#[cfg(not(target_os = "windows"))]
pub fn reduce_transparency() -> bool {
    false
}
