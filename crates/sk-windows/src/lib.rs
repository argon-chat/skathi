//! Generated Windows SDK bindings. Regenerate with the workspace's tools/windows-bindings.
#![cfg(windows)]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

#[cfg(any(feature = "runtime", feature = "win32"))]
pub use windows_core as core;

#[cfg(feature = "raw")]
#[rustfmt::skip]
pub mod raw;
#[cfg(feature = "win32")]
#[rustfmt::skip]
pub mod win32;
#[cfg(feature = "runtime")]
#[rustfmt::skip]
#[allow(dead_code)]
pub mod runtime;

#[cfg(feature = "runtime")]
type PropertyEntry =
    windows_collections::IKeyValuePair<windows_core::HSTRING, windows_core::IInspectable>;

#[cfg(feature = "runtime")]
impl runtime::DataPackagePropertySet {
    // The generated IntoIterator implementation needs the inherited IIterable method.
    fn First(&self) -> windows_core::Result<windows_collections::IIterator<PropertyEntry>> {
        use windows_core::Interface;
        self.cast::<windows_collections::IIterable<PropertyEntry>>()?
            .First()
    }
}
