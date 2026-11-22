//! `skathi:memory`: native memory that JS reads and writes without a copy and frees when it says so.
//!
//! The memory of a chunk is reference counted: the `NativeBuffer` that hands it out, the external
//! `ArrayBuffer` JS sees, a `NativePointer` taken from it and a library call in progress each hold
//! one count. `free()` detaches the `ArrayBuffer`, so every JS view of it reads as empty, and drops
//! the buffer's own count; the bytes go back to the allocator when nothing holds them any more.

use std::alloc::{self, Layout};
use std::ffi::c_void;
use std::fmt::Display;
use std::ptr::NonNull;
use std::sync::{Arc, OnceLock};

use sk_runtime::host;
use sk_runtime::redjs::ffi::RjsAbInfo;
use sk_runtime::redjs::glue;
use sk_runtime::redjs::webapi::util::*;

use super::support::{self, *};

const DEFAULT_ALIGNMENT: usize = 16;
const MAX_ALIGNMENT: usize = 65536;
const CHUNK_ALIGNMENT: usize = 64;
const DEFAULT_CHUNK: usize = 64 * 1024;

const BUFFER: &str = "skathi.buffer";
const ISSUED: &str = "skathi.issued";

struct Chunk {
    ptr: NonNull<u8>,
    layout: Layout,
}

// SAFETY: a chunk is plain bytes; who writes them when is the business of whoever holds a region.
unsafe impl Send for Chunk {}
unsafe impl Sync for Chunk {}

impl Chunk {
    fn new(size: usize, align: usize, zeroed: bool) -> Option<Chunk> {
        let layout = Layout::from_size_align(size.max(1), align).ok()?;
        // SAFETY: the layout's size is not zero.
        let raw = unsafe {
            if zeroed {
                alloc::alloc_zeroed(layout)
            } else {
                alloc::alloc(layout)
            }
        };
        NonNull::new(raw).map(|ptr| Chunk { ptr, layout })
    }

    fn size(&self) -> usize {
        self.layout.size()
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        // SAFETY: allocated in `Chunk::new` with this layout.
        unsafe { alloc::dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

/// Bytes of a chunk. Holding one keeps them allocated, whatever JS has freed.
#[derive(Clone)]
pub(super) struct Region {
    chunk: Arc<Chunk>,
    offset: usize,
    len: usize,
}

impl Region {
    pub fn ptr(&self) -> *mut u8 {
        // SAFETY: `offset + len` is within the chunk.
        unsafe { self.chunk.ptr.as_ptr().add(self.offset) }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// # Safety
    /// The bytes outlive the borrow only while some region of the chunk is held.
    pub unsafe fn bytes<'a>(&self) -> &'a [u8] {
        std::slice::from_raw_parts(self.ptr(), self.len)
    }
}

struct NativeBuffer {
    /// `None` once freed.
    region: Option<Region>,
    /// Handed to a call on another thread: the `ArrayBuffer` is detached until the call returns.
    away: bool,
}

static NATIVE_BUFFER: OnceLock<usize> = OnceLock::new();
static ARENA: OnceLock<usize> = OnceLock::new();

unsafe fn state<'a>(g: *mut c_void, v: u64) -> Option<&'a mut NativeBuffer> {
    host_data::<NativeBuffer>(
        g,
        support::host_class::<NativeBuffer>(&NATIVE_BUFFER, "NativeBuffer"),
        v,
    )
}

unsafe fn this_buffer<'a>(g: *mut c_void, this: u64) -> Js<&'a mut NativeBuffer> {
    let Some(native) = state(g, this) else {
        return type_error(g, "this is not a NativeBuffer");
    };
    sync(g, this, native);
    Ok(native)
}

unsafe fn detached(g: *mut c_void, buffer: u64) -> bool {
    let mut info = RjsAbInfo::default();
    glue::rjs_arraybuffer_info(g, buffer, &mut info) != 0 || info.detached != 0
}

/// A buffer whose `ArrayBuffer` JS detached itself, with `transfer()`, is freed as far as it is
/// concerned: the bytes now belong to the new `ArrayBuffer`.
unsafe fn sync(g: *mut c_void, this: u64, native: &mut NativeBuffer) {
    if native.region.is_some() && !native.away && detached(g, slot_get(g, this, BUFFER)) {
        native.region = None;
    }
}

unsafe extern "C" fn release(_bytes: *mut c_void, chunk: *mut c_void) {
    drop(Arc::from_raw(chunk as *const Chunk));
}

/// An `ArrayBuffer` over `region` that holds a count of its chunk until it is collected or detached.
unsafe fn external(g: *mut c_void, region: &Region) -> Option<u64> {
    let held = Arc::into_raw(region.chunk.clone()) as *mut c_void;
    let mut out = UNDEF;
    let release: unsafe extern "C" fn(*mut c_void, *mut c_void) = release;
    let status = sk_runtime::redjs::ffi::redjs_arraybuffer_external(
        g,
        region.ptr().cast(),
        region.len,
        release as *const c_void,
        held,
        &mut out,
    );
    if status != 0 {
        drop(Arc::from_raw(held as *const Chunk));
        return None;
    }
    Some(out)
}

unsafe fn issue(g: *mut c_void, region: Region, what: &str) -> Js<u64> {
    let Some(buffer) = external(g, &region) else {
        return range_error(
            g,
            &format!(
                "{what}: {} bytes are more than an ArrayBuffer holds",
                region.len
            ),
        );
    };
    let object = host_create(
        g,
        support::host_class::<NativeBuffer>(&NATIVE_BUFFER, "NativeBuffer"),
        prototype(g, "NativeBuffer"),
        NativeBuffer {
            region: Some(region),
            away: false,
        },
    );
    slot_set(g, object, BUFFER, buffer);
    Ok(object)
}

unsafe fn invalid<T>(g: *mut c_void, message: &str) -> Js<T> {
    host::throw(g, "InvalidStateError", message);
    Err(Thrown)
}

unsafe fn usable<'a>(
    g: *mut c_void,
    v: u64,
    what: &dyn Display,
) -> Js<Option<&'a mut NativeBuffer>> {
    let Some(native) = state(g, v) else {
        return Ok(None);
    };
    sync(g, v, native);
    if native.region.is_none() {
        return invalid(g, &format!("{what}: the NativeBuffer is freed"));
    }
    if native.away {
        return invalid(
            g,
            &format!(
                "{what}: the NativeBuffer is in use by a call on another thread; await it first"
            ),
        );
    }
    Ok(Some(native))
}

/// The memory of `v` when it is a `NativeBuffer`, `None` for any other value. A freed buffer, or
/// one a call on another thread has, throws `InvalidStateError`.
pub(super) unsafe fn region_of(g: *mut c_void, v: u64, what: &dyn Display) -> Js<Option<Region>> {
    Ok(usable(g, v, what)?.and_then(|native| native.region.clone()))
}

/// The `ArrayBuffer` of `v` when it is a `NativeBuffer`, with the same errors as [`region_of`].
pub(super) unsafe fn buffer_of(g: *mut c_void, v: u64, what: &dyn Display) -> Js<Option<u64>> {
    Ok(usable(g, v, what)?.map(|_| slot_get(g, v, BUFFER)))
}

/// Throws `InvalidStateError` unless the `NativeBuffer` `v` is usable and `buffer` is still its
/// `ArrayBuffer`: a view made before it went to another thread is as dead as a transferred one.
pub(super) unsafe fn check_current(
    g: *mut c_void,
    v: u64,
    buffer: u64,
    what: &dyn Display,
) -> Js<()> {
    if usable(g, v, what)?.is_some() && slot_get(g, v, BUFFER) != buffer {
        return invalid(
            g,
            &format!("{what}: the NativeBuffer went to a call on another thread since; make a new one over it"),
        );
    }
    Ok(())
}

/// Hands the memory of a `NativeBuffer` to a call on another thread. Its `ArrayBuffer` is
/// detached, as a transfer would, until [`come_back`].
pub(super) unsafe fn send_away(g: *mut c_void, v: u64, what: &dyn Display) -> Js<Region> {
    let Some(native) = usable(g, v, what)? else {
        return type_error(g, &format!("{what} must be a NativeBuffer"));
    };
    let Some(region) = native.region.clone() else {
        return Err(Thrown);
    };
    if glue::rjs_arraybuffer_detach(g, slot_get(g, v, BUFFER)) != 0 {
        return invalid(
            g,
            &format!("{what}: the NativeBuffer's ArrayBuffer cannot be detached"),
        );
    }
    native.away = true;
    Ok(region)
}

/// The call on another thread returned: a buffer not freed meanwhile gets a new `ArrayBuffer`.
pub(super) unsafe fn come_back(g: *mut c_void, v: u64) {
    let Some(native) = state(g, v) else {
        return;
    };
    native.away = false;
    let Some(region) = native.region.clone() else {
        return;
    };
    match external(g, &region) {
        Some(buffer) => slot_set(g, v, BUFFER, buffer),
        None => native.region = None,
    }
}

unsafe fn alignment(g: *mut c_void, v: u64, what: &str) -> Js<usize> {
    if v == UNDEF {
        return Ok(DEFAULT_ALIGNMENT);
    }
    let align = count(g, v, what)?;
    if !align.is_power_of_two() || align > MAX_ALIGNMENT {
        return range_error(
            g,
            &format!("{what} must be a power of two from 1 to {MAX_ALIGNMENT}"),
        );
    }
    Ok(align)
}

unsafe fn report(g: *mut c_void, bytes: usize) {
    glue::rjs_report_extra_memory(g, bytes as u64);
}

std_fn!(fn native_alloc(g, args) {
    let what = "NativeBuffer.alloc";
    let len = count(g, args.arg(0).0, "NativeBuffer.alloc: the byte length")?;
    let options = args.arg(1).0;
    let zeroed = match option(g, options, "zeroed") {
        UNDEF => true,
        v => to_bool(g, v),
    };
    let align = alignment(g, option(g, options, "alignment"), "NativeBuffer.alloc: alignment")?;
    let Some(chunk) = Chunk::new(len, align, zeroed) else {
        return range_error(g, &format!("{what}: {len} bytes could not be allocated"));
    };
    report(g, chunk.size());
    issue(g, Region { chunk: Arc::new(chunk), offset: 0, len }, what)
});

std_fn!(fn native_byte_length(g, args) {
    let native = this_buffer(g, args.this().0)?;
    Ok(enc_f64(native.region.as_ref().map_or(0, Region::len) as f64))
});

std_fn!(fn native_buffer(g, args) {
    let this = args.this().0;
    this_buffer(g, this)?;
    Ok(slot_get(g, this, BUFFER))
});

std_fn!(fn native_freed(g, args) {
    Ok(enc_bool(this_buffer(g, args.this().0)?.region.is_none()))
});

std_fn!(fn native_free(g, args) {
    let this = args.this().0;
    let native = this_buffer(g, this)?;
    if native.region.is_some() && !native.away {
        glue::rjs_arraybuffer_detach(g, slot_get(g, this, BUFFER));
    }
    native.region = None;
    Ok(UNDEF)
});

/// A bump allocator over chunks it keeps from one `reset()` to the next.
struct ArenaState {
    chunk_size: usize,
    chunks: Vec<Arc<Chunk>>,
    index: usize,
    offset: usize,
    used: usize,
}

impl ArenaState {
    fn new(chunk_size: usize) -> Self {
        Self {
            chunk_size,
            chunks: Vec::new(),
            index: 0,
            offset: 0,
            used: 0,
        }
    }

    /// `len` zeroed bytes at a multiple of `align`, and the size of a chunk made for them, if any.
    fn take(&mut self, len: usize, align: usize) -> Option<(Region, usize)> {
        let mut made = 0;
        loop {
            if let Some(chunk) = self.chunks.get(self.index) {
                let base = chunk.ptr.as_ptr() as usize;
                let start = base
                    .checked_add(self.offset)?
                    .checked_next_multiple_of(align)?
                    - base;
                if start.checked_add(len)? <= chunk.size() {
                    let region = Region {
                        chunk: chunk.clone(),
                        offset: start,
                        len,
                    };
                    // SAFETY: the region is within the chunk.
                    unsafe { std::ptr::write_bytes(region.ptr(), 0, len) };
                    self.offset = start + len;
                    self.used += len;
                    return Some((region, made));
                }
                self.index += 1;
                self.offset = 0;
                continue;
            }
            let size = self.chunk_size.max(len.checked_add(align - 1)?);
            let chunk = Chunk::new(size, CHUNK_ALIGNMENT, true)?;
            made = chunk.size();
            self.chunks.push(Arc::new(chunk));
        }
    }

    /// Starts over. A chunk something still holds, a pointer or a call in progress, is let go
    /// rather than handed out again.
    fn reset(&mut self) {
        self.chunks.retain(|chunk| Arc::strong_count(chunk) == 1);
        self.index = 0;
        self.offset = 0;
        self.used = 0;
    }
}

unsafe fn arena<'a>(g: *mut c_void, this: u64) -> Js<&'a mut ArenaState> {
    match host_data::<ArenaState>(g, support::host_class::<ArenaState>(&ARENA, "Arena"), this) {
        Some(state) => Ok(state),
        None => type_error(g, "this is not an Arena"),
    }
}

std_fn!(fn arena_new(g, args) {
    let chunk_size = match option(g, args.arg(0).0, "chunkSize") {
        UNDEF => DEFAULT_CHUNK,
        v => count(g, v, "Arena: chunkSize")?.max(1),
    };
    let class = support::host_class::<ArenaState>(&ARENA, "Arena");
    let object = host_create(g, class, prototype(g, "Arena"), ArenaState::new(chunk_size));
    slot_set(g, object, ISSUED, new_array(g));
    Ok(object)
});

std_fn!(fn arena_alloc(g, args) {
    let this = args.this().0;
    let what = "Arena.alloc";
    let len = count(g, args.arg(0).0, "Arena.alloc: the byte length")?;
    let align = alignment(g, args.arg(1).0, "Arena.alloc: the alignment")?;
    let Some((region, made)) = arena(g, this)?.take(len, align) else {
        return range_error(g, &format!("{what}: {len} bytes could not be allocated"));
    };
    if made > 0 {
        report(g, made);
    }
    let object = issue(g, region, what)?;
    let issued = slot_get(g, this, ISSUED);
    set_i(g, issued, array_len(g, issued), object);
    Ok(object)
});

std_fn!(fn arena_used(g, args) {
    Ok(enc_f64(arena(g, args.this().0)?.used as f64))
});

std_fn!(fn arena_reset(g, args) {
    let this = args.this().0;
    arena(g, this)?;
    let issued = slot_get(g, this, ISSUED);
    for i in 0..array_len(g, issued) {
        let object = get_i(g, issued, i);
        if let Some(native) = state(g, object) {
            if native.region.is_some() && !native.away {
                glue::rjs_arraybuffer_detach(g, slot_get(g, object, BUFFER));
            }
            native.region = None;
        }
    }
    slot_set(g, this, ISSUED, new_array(g));
    arena(g, this)?.reset();
    Ok(UNDEF)
});

#[derive(Clone, Copy)]
enum Element {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
    F64,
}

const ELEMENTS: &[&str] = &["u8", "i8", "u16", "i16", "u32", "i32", "f32", "f64"];
const ELEMENT_KINDS: &[Element] = &[
    Element::U8,
    Element::I8,
    Element::U16,
    Element::I16,
    Element::U32,
    Element::I32,
    Element::F32,
    Element::F64,
];

impl Element {
    fn size(self) -> usize {
        match self {
            Element::U8 | Element::I8 => 1,
            Element::U16 | Element::I16 => 2,
            Element::U32 | Element::I32 | Element::F32 => 4,
            Element::F64 => 8,
        }
    }

    /// `n` in little-endian bytes, converted as `DataView`'s setters convert it.
    fn store(self, n: f64, out: &mut [u8]) {
        let int = if n.is_finite() {
            let m = n.trunc() % 4_294_967_296.0;
            (if m < 0.0 { m + 4_294_967_296.0 } else { m }) as u32
        } else {
            0
        };
        match self {
            Element::U8 | Element::I8 => out[0] = int as u8,
            Element::U16 | Element::I16 => out[..2].copy_from_slice(&(int as u16).to_le_bytes()),
            Element::U32 | Element::I32 => out[..4].copy_from_slice(&int.to_le_bytes()),
            Element::F32 => out[..4].copy_from_slice(&(n as f32).to_le_bytes()),
            Element::F64 => out[..8].copy_from_slice(&n.to_le_bytes()),
        }
    }
}

// `copyInto(view, type, stride, length, values, start)`: `MemoryView.copyFrom`, with the view's
// `DataView` starting at element 0.
std_fn!(fn copy_into(g, args) {
    let what = "MemoryView.copyFrom";
    let index = choice(g, args.arg(1).0, ELEMENTS, usize::MAX, "MemoryView.copyFrom: the type")?;
    let Some(element) = ELEMENT_KINDS.get(index).copied() else {
        return type_error(g, "MemoryView.copyFrom: the type is missing");
    };
    let stride = count(g, args.arg(2).0, "MemoryView.copyFrom: the stride")?;
    let length = count(g, args.arg(3).0, "MemoryView.copyFrom: the length")?;
    let values = super::binary::numbers(g, args.arg(4).0, "MemoryView.copyFrom: the values")?;
    let start = match args.arg(5).0 {
        UNDEF => 0,
        v => count(g, v, "MemoryView.copyFrom: start")?,
    };
    if start > length || values.len() > length - start {
        return range_error(
            g,
            &format!("{what}: {} values from {start} do not fit the {length} elements of the view", values.len()),
        );
    }
    if values.is_empty() {
        return Ok(UNDEF);
    }
    // Read after the values: converting them may have run code that freed the memory.
    let Some(view) = typed_view(g, args.arg(0).0).filter(|v| v.kind == TA_DATA_VIEW) else {
        return type_error(g, "MemoryView.copyFrom: this is not a MemoryView");
    };
    let size = element.size();
    let last = (start + values.len() - 1).checked_mul(stride).and_then(|at| at.checked_add(size));
    if last.is_none_or(|end| end > view.len) {
        return invalid(g, &format!("{what}: the memory under the view was freed or detached"));
    }
    let bytes = view.bytes_mut();
    for (i, value) in values.into_iter().enumerate() {
        let at = (start + i) * stride;
        element.store(value, &mut bytes[at..at + size]);
    }
    Ok(UNDEF)
});

pub(super) unsafe fn install(g: *mut c_void, std: u64) {
    let native = support::class(
        g,
        &ClassSpec {
            name: "NativeBuffer",
            arity: 0,
            construct: None,
            parent_proto: 0,
            methods: &[("free", 0, native_free)],
            getters: &[
                ("byteLength", native_byte_length),
                ("buffer", native_buffer),
                ("freed", native_freed),
            ],
            accessors: &[],
        },
    );
    static_fn(g, native.ctor, "alloc", 1, native_alloc);
    let arena = support::class(
        g,
        &ClassSpec {
            name: "Arena",
            arity: 0,
            construct: Some(arena_new),
            parent_proto: 0,
            methods: &[("alloc", 1, arena_alloc), ("reset", 0, arena_reset)],
            getters: &[("used", arena_used)],
            accessors: &[],
        },
    );

    let module = new_object(g, UNDEF);
    define(g, module, "NativeBuffer", native.ctor, 0);
    define(g, module, "Arena", arena.ctor, 0);
    static_fn(g, module, "copyInto", 6, copy_into);
    define(g, std, "memory", module, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_aligns_and_zeroes_what_it_hands_out() {
        let mut arena = ArenaState::new(256);
        let (a, made) = arena.take(3, 1).unwrap();
        assert_eq!(made, 256);
        unsafe { std::ptr::write_bytes(a.ptr(), 0xAA, 3) };
        let (b, made) = arena.take(8, 64).unwrap();
        assert_eq!(made, 0);
        assert_eq!(b.ptr() as usize % 64, 0);
        assert!(b.offset >= a.offset + a.len);
        let address = a.ptr();
        drop((a, b));
        arena.reset();
        let (again, _) = arena.take(3, 1).unwrap();
        assert_eq!(again.ptr(), address);
        assert_eq!(unsafe { again.bytes() }, [0, 0, 0]);
    }

    #[test]
    fn arena_spills_into_a_chunk_big_enough() {
        let mut arena = ArenaState::new(64);
        arena.take(40, 1).unwrap();
        let (big, made) = arena.take(1000, 256).unwrap();
        assert!(made >= 1000);
        assert_eq!(big.ptr() as usize % 256, 0);
        assert_eq!(arena.used, 1040);
        assert_eq!(arena.chunks.len(), 2);
    }

    #[test]
    fn reset_reuses_free_chunks_and_lets_held_ones_go() {
        let mut arena = ArenaState::new(128);
        let (first, _) = arena.take(16, 16).unwrap();
        let address = first.ptr();
        drop(first);
        arena.reset();
        let (reused, _) = arena.take(16, 16).unwrap();
        assert_eq!(reused.ptr(), address);
        arena.reset();
        let (fresh, made) = arena.take(16, 16).unwrap();
        assert!(made > 0);
        assert_ne!(fresh.ptr(), address);
        unsafe { std::ptr::write_bytes(reused.ptr(), 0x55, 16) };
        assert_eq!(unsafe { reused.bytes() }, [0x55; 16]);
    }
}
