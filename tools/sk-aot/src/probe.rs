//! Probes of engine options and of what a host sees: `image --option WORD` and `run`.

use core::ffi::{c_char, c_void};
use redjs::api::{Context, Error};
use std::path::Path;

#[repr(C)]
struct Bytes {
    ptr: *const u8,
    len: usize,
}

unsafe extern "C" {
    fn sk_aot_compile_image(
        program: *const u8,
        length: usize,
        identity: *const u8,
        words: *const Bytes,
        count: usize,
    ) -> *mut c_void;
    fn sk_aot_image_bytes(image: *mut c_void, bytes: *mut *const u8) -> usize;
    fn sk_aot_image_entries(image: *mut c_void, entries: *mut *const u32) -> usize;
    fn sk_aot_image_diagnostic(image: *mut c_void, bytes: *mut *const c_char) -> usize;
    fn sk_aot_image_free(image: *mut c_void);
}

/// redjs's identity, over its compile options followed by the words.
fn identity(program: &[u8], words: &[String]) -> [u8; 16] {
    let mut input = b"skathi-aot-image-v1\0".to_vec();
    input.extend_from_slice(include_bytes!("../../../vendor/redjs/redjs/pins.txt"));
    input.extend_from_slice(redjs::aot::BUILD_TABLE_ID.as_bytes());
    input.extend_from_slice(redjs::aot::COMPILE_OPTIONS.as_bytes());
    for word in words {
        input.push(b';');
        input.extend_from_slice(word.as_bytes());
    }
    input.extend_from_slice(program);
    let hex = redjs::aot::digest_hex(&input);
    std::array::from_fn(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap())
}

pub fn compile(program: &[u8], words: &[String]) -> Result<redjs::aot::CompiledImage, String> {
    let identity = identity(program, words);
    let raw: Vec<Bytes> = words
        .iter()
        .map(|word| Bytes {
            ptr: word.as_ptr(),
            len: word.len(),
        })
        .collect();
    let handle = unsafe {
        sk_aot_compile_image(
            program.as_ptr(),
            program.len(),
            identity.as_ptr(),
            raw.as_ptr(),
            raw.len(),
        )
    };
    if handle.is_null() {
        return Err("the engine did not initialize".into());
    }
    let mut text = std::ptr::null();
    let length = unsafe { sk_aot_image_diagnostic(handle, &mut text) };
    let diagnostics = match length {
        0 => String::new(),
        _ => String::from_utf8_lossy(unsafe {
            std::slice::from_raw_parts(text.cast::<u8>(), length)
        })
        .into_owned(),
    };
    let mut bytes = std::ptr::null();
    let length = unsafe { sk_aot_image_bytes(handle, &mut bytes) };
    let image =
        (length != 0).then(|| unsafe { std::slice::from_raw_parts(bytes, length) }.to_vec());
    let mut entries = std::ptr::null();
    let count = unsafe { sk_aot_image_entries(handle, &mut entries) };
    let entries = match count {
        0 => Vec::new(),
        _ => unsafe { std::slice::from_raw_parts(entries, count) }.to_vec(),
    };
    unsafe { sk_aot_image_free(handle) };
    match image {
        Some(bytes) => Ok(redjs::aot::CompiledImage {
            bytes,
            entries,
            identity,
            diagnostics,
        }),
        None => Err(format!("the engine made no image: {diagnostics}")),
    }
}

/// Runs an image's first entry the way the sk host does: preludes evaluated as text, then
/// `__SK_AOT_HOST__` bound to globalThis. Prints the global `__sk_aot_probe` afterwards.
pub fn run(path: &str, preludes: &[String]) -> Result<(), String> {
    let identity: [u8; 16] = std::fs::read(format!("{path}.identity"))
        .map_err(|error| format!("cannot read {path}.identity: {error}"))?
        .try_into()
        .map_err(|_| "the identity is not 16 bytes")?;
    unsafe { redjs::aot::load_image(Path::new(path), identity) }?;
    let context = Context::new();
    let failed = |error: Error| match error {
        Error::Exception(value) => context
            .to_rust_string(value)
            .unwrap_or_else(|_| "JS exception".into()),
        other => other.to_string(),
    };
    for prelude in preludes {
        let text = std::fs::read_to_string(prelude)
            .map_err(|error| format!("cannot read {prelude}: {error}"))?;
        context.eval_named(&text, prelude).map_err(failed)?;
    }
    let host = context.global_get("globalThis").map_err(failed)?;
    context
        .global_set("__SK_AOT_HOST__", host)
        .map_err(failed)?;
    let entries = std::fs::read_to_string(format!("{path}.entries"))
        .map_err(|error| format!("cannot read {path}.entries: {error}"))?;
    let entry: u32 = entries
        .lines()
        .next()
        .and_then(|line| line.parse().ok())
        .ok_or("no entry")?;
    match context.run_aot_module(entry).map_err(failed)? {
        redjs::aot::ModuleEvaluation::Fulfilled(_) => {}
        redjs::aot::ModuleEvaluation::Pending(_) => return Err("the entry awaits host work".into()),
    }
    let probe = context.global_get("__sk_aot_probe").map_err(failed)?;
    println!(
        "aot_active={} probe={}",
        redjs::aot::is_active(),
        context.to_rust_string(probe).map_err(failed)?
    );
    Ok(())
}
