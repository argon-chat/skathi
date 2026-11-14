//! The system request behind the Screen Wake Lock API: a power request on Windows, an IOKit assertion
//! on macOS. Both keep the display on and so the system awake.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "macos")]
pub use mac::hold_screen;
#[cfg(target_os = "windows")]
pub use win::hold_screen;
