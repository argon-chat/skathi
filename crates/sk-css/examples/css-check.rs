//! Headless CSS diagnostics shipped with the CLI.

#[cfg(target_arch = "wasm32")]
mod wasm {
    #[link(wasm_import_module = "sk")]
    extern "C" {
        fn read(ptr: *mut u8, len: usize);
        fn write(ptr: *const u8, len: usize);
    }

    #[no_mangle]
    pub extern "C" fn check(len: usize) {
        let mut input = vec![0; len];
        // SAFETY: the synchronous JS imports copy exactly len bytes to/from this live allocation.
        unsafe { read(input.as_mut_ptr(), len) };
        let block = serde_json::from_slice(&input).unwrap();
        let result = sk_css::diagnostics::inspect(&block);
        let output = serde_json::to_vec(&result).unwrap();
        unsafe { write(output.as_ptr(), output.len()) };
    }
}
