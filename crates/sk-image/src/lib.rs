//! `skimage.dll`: no code and no exports. `sk build` appends a native image to a copy of it as
//! sections of their own, so the image travels as a PE file that can be signed and keeps its
//! bytes, and its signature, from one release to the next while the image is unchanged.
#![no_std]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
