//! `navigator.maxTouchPoints`: what the touch hardware reports, 0 on a platform whose touch input
//! the host does not deliver yet.

use sk_runtime::redjs::webapi::navigator;

pub fn install() {
    #[cfg(target_os = "windows")]
    navigator::set_max_touch_points(Some(max_touch_points));
    #[cfg(not(target_os = "windows"))]
    navigator::set_max_touch_points(None);
}

#[cfg(target_os = "windows")]
fn max_touch_points() -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_MAXIMUMTOUCHES};
    // SAFETY: no pointers.
    let n = unsafe { GetSystemMetrics(SM_MAXIMUMTOUCHES) };
    n.max(0) as u32
}
