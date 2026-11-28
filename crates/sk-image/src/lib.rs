//! The empty signed-code container `sk build` fills with a native AOT image.
#![no_std]

#[cfg(target_os = "macos")]
#[unsafe(no_mangle)]
pub static skathi_image_anchor: u8 = 0;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
