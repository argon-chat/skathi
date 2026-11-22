//! `skathi::NativeBuffer`: memory a native module and the app hand each other without a copy. In
//! the app it is the `NativeBuffer` of `skathi:memory`; the memory model is that module's.
//!
//! Rust and JS never reach the bytes at the same time. A buffer returned to the app is the app's:
//! one Rust still holds afterwards (returned by reference) is copied for it. A buffer the app
//! passes is lent: for a function's call, or detached in the app until a task settles. One Rust
//! keeps past that is Rust's from then on, and freed as far as the app is concerned.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::ser::{Serialize, Serializer};

use super::marshal::Error;
use crate::js::{Context, Value};
use crate::stdlib::memory::{self, Held, Region};

pub(super) const TOKEN: &str = "\0skathi::NativeBuffer";

/// Bytes the app sees as a `NativeBuffer` of `skathi:memory`, handed over without a copy either way.
/// Made from a `Vec<u8>` or `Box<[u8]>` in their own memory, or by [`NativeBuffer::zeroed`].
pub struct NativeBuffer {
    held: Held,
    /// Only counted: while it lives, the app's buffer it was lent from stays lent.
    _lease: Option<Arc<()>>,
}

impl NativeBuffer {
    /// `len` zeroed bytes at an address that is a multiple of 16, as `NativeBuffer.alloc` gives.
    pub fn zeroed(len: usize) -> Self {
        Self::own(Region::zeroed(len))
    }

    fn own(region: Region) -> Self {
        Self {
            held: Held::new(region),
            _lease: None,
        }
    }
}

impl From<Vec<u8>> for NativeBuffer {
    fn from(v: Vec<u8>) -> Self {
        Self::own(Region::from_vec(v))
    }
}

impl From<Box<[u8]>> for NativeBuffer {
    fn from(v: Box<[u8]>) -> Self {
        Self::from(v.into_vec())
    }
}

impl Deref for NativeBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // SAFETY: `held` keeps the chunk; JS cannot reach these bytes while this value exists
        // (see the module's comment).
        unsafe { self.held.region().bytes() }
    }
}

impl DerefMut for NativeBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: as for `deref`, and `&mut self` is the only way to these bytes in Rust.
        unsafe { self.held.region().bytes_mut() }
    }
}

impl AsRef<[u8]> for NativeBuffer {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl AsMut<[u8]> for NativeBuffer {
    fn as_mut(&mut self) -> &mut [u8] {
        self
    }
}

impl fmt::Debug for NativeBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NativeBuffer({} bytes)", self.len())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A function's call: arguments are lent, results made.
    Call,
    /// A task's arguments, read in its call.
    Task,
    /// A task's result, built when it settles.
    Settle,
}

/// A `NativeBuffer` of the app that Rust code was given.
pub(crate) struct Lent {
    object: Value,
    region: Region,
    lease: Arc<()>,
    returned: bool,
}

impl Lent {
    fn kept(&self) -> bool {
        Arc::strong_count(&self.lease) > 1
    }
}

struct Borrow {
    start: usize,
    end: usize,
    mutable: bool,
}

struct Frame {
    kind: Kind,
    lent: Vec<Lent>,
    borrows: Vec<Borrow>,
    /// Chunks under the slices the call borrows, kept for its length.
    holds: Vec<Region>,
    /// `NativeBuffer`s made for a function's result, protected until the call returns.
    made: Vec<(Value, Region)>,
}

thread_local! {
    static FRAMES: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
    /// A result is being made: a buffer in it goes to JS as it is rather than as a copy.
    static MOVING: Cell<bool> = const { Cell::new(false) };
    static OUTGOING: RefCell<Option<(Region, bool)>> = const { RefCell::new(None) };
    static INCOMING: RefCell<Option<NativeBuffer>> = const { RefCell::new(None) };
}

fn framed<T>(kind: Kind, lent: Vec<Lent>, run: impl FnOnce() -> T) -> (T, Frame) {
    struct Pop(bool);
    impl Drop for Pop {
        fn drop(&mut self) {
            if self.0 {
                FRAMES.with(|f| f.borrow_mut().pop());
            }
        }
    }
    FRAMES.with(|f| {
        f.borrow_mut().push(Frame {
            kind,
            lent,
            borrows: Vec::new(),
            holds: Vec::new(),
            made: Vec::new(),
        })
    });
    let mut pushed = Pop(true);
    let out = run();
    pushed.0 = false;
    let frame = FRAMES.with(|f| f.borrow_mut().pop());
    (out, frame.expect("the frame pushed above"))
}

fn kind() -> Option<Kind> {
    FRAMES.with(|f| f.borrow().last().map(|f| f.kind))
}

/// Runs `run` with buffers in what it serializes handed over (`true`) or copied (`false`).
pub(crate) fn moving<T>(on: bool, run: impl FnOnce() -> T) -> T {
    let before = MOVING.with(|m| m.replace(on));
    let out = run();
    MOVING.with(|m| m.set(before));
    out
}

/// Runs a function's call on the JS thread. Then a lent buffer Rust kept is freed for JS, and a
/// buffer made for JS that Rust still holds becomes a copy.
pub(crate) fn call<T>(context: &Context, run: impl FnOnce() -> T) -> T {
    let (out, frame) = framed(Kind::Call, Vec::new(), || moving(true, run));
    let g = context.raw();
    for lent in &frame.lent {
        if lent.kept() {
            // SAFETY: the argument is a live NativeBuffer on this call's stack.
            unsafe {
                if lent.returned {
                    memory::rehome(g, lent.object.0)
                } else {
                    memory::keep(g, lent.object.0)
                }
            }
        }
    }
    for (object, region) in frame.made {
        if region.rust_holders() > 0 {
            // SAFETY: protected since it was made.
            unsafe { memory::rehome(g, object.0) };
        }
        context.unprotect(object);
    }
    out
}

/// Reads a task's arguments; what it returns lent goes back with [`settle`].
pub(crate) fn prepare<T>(run: impl FnOnce() -> T) -> (T, Vec<Lent>) {
    let (out, frame) = framed(Kind::Task, Vec::new(), run);
    (out, frame.lent)
}

/// A task settled: the buffers it was lent come back to JS, unless it kept them, and `build`
/// makes its result, in which a buffer it was lent is the app's own object again.
pub(crate) fn settle<T>(context: &Context, lent: Vec<Lent>, build: impl FnOnce() -> T) -> T {
    let g = context.raw();
    for l in &lent {
        // SAFETY: protected since the task was called.
        unsafe {
            if l.kept() {
                memory::keep(g, l.object.0)
            } else {
                memory::come_back(g, l.object.0)
            }
        }
    }
    let (out, frame) = framed(Kind::Settle, lent, build);
    for l in frame.lent {
        context.unprotect(l.object);
    }
    out
}

/// The page went away with the task's promise: nothing will come back to it.
pub(crate) fn abandon(context: &Context, lent: Vec<Lent>) {
    for l in lent {
        context.unprotect(l.object);
    }
}

fn invalid(context: &Context, problem: &str) -> Error {
    // SAFETY: plain FFI on the JS thread.
    Error::Thrown(Value(unsafe {
        sk_runtime::host::error(context.raw(), "InvalidStateError", problem)
    }))
}

fn register(start: *const u8, len: usize, mutable: bool) -> Result<(), Error> {
    if len == 0 {
        return Ok(());
    }
    let (start, end) = (start as usize, start as usize + len);
    FRAMES.with(|f| {
        let mut frames = f.borrow_mut();
        let clash = frames.iter().flat_map(|f| &f.borrows).any(|b| {
            b.start < end && start < b.end && (mutable || b.mutable)
        });
        if clash {
            return Err(Error::Message(
                "its memory is also in another argument, and one of them is written: pass separate buffers"
                    .into(),
            ));
        }
        if let Some(frame) = frames.last_mut() {
            frame.borrows.push(Borrow {
                start,
                end,
                mutable,
            });
        }
        Ok(())
    })
}

fn hold(region: Region) {
    FRAMES.with(|f| {
        if let Some(frame) = f.borrow_mut().last_mut() {
            frame.holds.push(region);
        }
    });
}

/// Whether slices are borrowed in place now: in a function's call, not a task's.
pub(super) fn in_call() -> bool {
    kind() == Some(Kind::Call)
}

/// Bytes of a view or an `ArrayBuffer` a function borrows: checked against the call's other
/// borrows, and a `NativeBuffer`'s chunk under them kept until the call returns.
pub(super) fn borrow(start: *const u8, len: usize, mutable: bool) -> Result<(), Error> {
    if !in_call() {
        return Ok(());
    }
    register(start, len, mutable)?;
    if let Some(chunk) = memory::holder_of(start) {
        hold(chunk);
    }
    Ok(())
}

/// The memory of a `NativeBuffer` argument a function borrows; `None` for any other value.
pub(super) fn lent_bytes(
    context: &Context,
    v: Value,
    mutable: bool,
) -> Result<Option<(*mut u8, usize)>, Error> {
    let Some(region) = bytes_of(context, v)? else {
        return Ok(None);
    };
    let found = (region.ptr(), region.len());
    register(found.0, found.1, mutable)?;
    hold(region);
    Ok(Some(found))
}

/// The memory of a `NativeBuffer` argument, to copy at once; `None` for any other value.
pub(super) fn bytes_of(context: &Context, v: Value) -> Result<Option<Region>, Error> {
    if !v.is_cell() {
        return Ok(None);
    }
    // SAFETY: plain FFI on a live argument.
    unsafe { memory::lend(context.raw(), v.0) }.map_err(|problem| invalid(context, problem))
}

/// Reads a `NativeBuffer` parameter from `v` into [`INCOMING`]; `false` for any other value.
pub(super) fn read(context: &Context, v: Value) -> Result<bool, Error> {
    if !v.is_cell() {
        return Ok(false);
    }
    let g = context.raw();
    let kind = kind();
    // SAFETY: plain FFI on a live argument.
    let region =
        match kind {
            Some(Kind::Call) => unsafe { memory::lend(g, v.0) },
            Some(Kind::Task) => unsafe { memory::send(g, v.0) },
            _ => return Err(Error::Message(
                "a NativeBuffer is read only as an argument of a native module's function or task"
                    .into(),
            )),
        };
    let region = match region {
        Ok(Some(region)) => region,
        Ok(None) => return Ok(false),
        Err(problem) => return Err(invalid(context, problem)),
    };
    if kind == Some(Kind::Call) {
        register(region.ptr(), region.len(), true)?;
    } else {
        context.protect(v);
    }
    let lease = Arc::new(());
    FRAMES.with(|f| {
        if let Some(frame) = f.borrow_mut().last_mut() {
            frame.lent.push(Lent {
                object: v,
                region: region.clone(),
                lease: lease.clone(),
                returned: false,
            });
        }
    });
    INCOMING.with(|i| {
        *i.borrow_mut() = Some(NativeBuffer {
            held: Held::new(region),
            _lease: Some(lease),
        })
    });
    Ok(true)
}

pub(super) fn discard_incoming() {
    INCOMING.with(|i| i.borrow_mut().take());
}

/// What a result's serializer takes for the `NativeBuffer` it is at: its memory, or a copy.
pub(super) fn outgoing() -> Option<Region> {
    let (region, moving) = OUTGOING.with(|o| o.borrow_mut().take())?;
    Some(if moving {
        region
    } else {
        // SAFETY: the NativeBuffer being serialized holds the chunk and is borrowed meanwhile.
        Region::copy_of(unsafe { region.bytes() })
    })
}

/// The app's value for `region` of a result.
pub(super) fn to_js(context: &Context, region: Region) -> Result<Value, Error> {
    enum Plan {
        Lent(Value),
        Made,
        Plain,
    }
    let plan = FRAMES.with(|f| {
        let mut frames = f.borrow_mut();
        let Some(frame) = frames.last_mut() else {
            return Plan::Plain;
        };
        let settling = frame.kind == Kind::Settle;
        if let Some(lent) = frame
            .lent
            .iter_mut()
            .find(|l| l.region.same(&region) && !(settling && l.kept()))
        {
            lent.returned = true;
            return Plan::Lent(lent.object);
        }
        if frame.kind == Kind::Call {
            Plan::Made
        } else {
            Plan::Plain
        }
    });
    let region = match plan {
        Plan::Lent(object) => return Ok(object),
        Plan::Made => region,
        // SAFETY: Rust code holds the chunk, and can only read it while it holds it so.
        Plan::Plain if region.rust_holders() > 0 => Region::copy_of(unsafe { region.bytes() }),
        Plan::Plain => region,
    };
    let len = region.len();
    // SAFETY: plain FFI on the JS thread.
    let Some(object) = (unsafe { memory::make(context.raw(), region.clone()) }) else {
        return Err(Error::Message(format!(
            "{len} bytes are more than an ArrayBuffer holds"
        )));
    };
    let object = Value(object);
    if matches!(plan, Plan::Made) {
        context.protect(object);
        FRAMES.with(|f| {
            if let Some(frame) = f.borrow_mut().last_mut() {
                frame.made.push((object, region));
            }
        });
    }
    Ok(object)
}

struct AsBytes<'a>(&'a [u8]);

impl Serialize for AsBytes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.0)
    }
}

impl Serialize for NativeBuffer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let moving = MOVING.with(Cell::get);
        OUTGOING.with(|o| *o.borrow_mut() = Some((self.held.region().clone(), moving)));
        let result = serializer.serialize_newtype_struct(TOKEN, &AsBytes(self));
        OUTGOING.with(|o| o.borrow_mut().take());
        result
    }
}

struct Incoming;

impl<'de> Visitor<'de> for Incoming {
    type Value = NativeBuffer;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a NativeBuffer of skathi:memory")
    }

    fn visit_unit<E: de::Error>(self) -> Result<NativeBuffer, E> {
        INCOMING
            .with(|i| i.borrow_mut().take())
            .ok_or_else(|| E::invalid_type(de::Unexpected::Unit, &self))
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<NativeBuffer, E> {
        Ok(NativeBuffer::from(v.to_vec()))
    }

    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<NativeBuffer, E> {
        Ok(NativeBuffer::from(v))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<NativeBuffer, A::Error> {
        let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1 << 20));
        while let Some(b) = seq.next_element::<u8>()? {
            out.push(b);
        }
        Ok(NativeBuffer::from(out))
    }

    fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<NativeBuffer, D::Error> {
        d.deserialize_byte_buf(self)
    }
}

impl<'de> serde::Deserialize<'de> for NativeBuffer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_newtype_struct(TOKEN, Incoming)
    }
}

#[cfg(feature = "types")]
impl specta::Type for NativeBuffer {
    fn definition(_: &mut specta::Types) -> specta::datatype::DataType {
        specta_typescript::define("NativeBuffer").into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vec_becomes_a_buffer_in_its_own_memory() {
        let mut v = Vec::with_capacity(64);
        v.extend_from_slice(&[1, 2, 3]);
        let address = v.as_ptr();
        let buffer = NativeBuffer::from(v);
        assert_eq!(buffer.as_ptr(), address);
        assert_eq!(&*buffer, &[1, 2, 3]);
        let boxed = NativeBuffer::from(vec![9u8; 5].into_boxed_slice());
        assert_eq!(&*boxed, &[9; 5]);
        assert!(NativeBuffer::from(Vec::new()).is_empty());
    }

    #[test]
    fn zeroed_buffers_are_aligned_like_the_ones_js_allocates() {
        let mut buffer = NativeBuffer::zeroed(100);
        assert_eq!(buffer.as_ptr() as usize % 16, 0);
        assert!(buffer.iter().all(|&b| b == 0));
        buffer[99] = 7;
        assert_eq!(buffer[99], 7);
    }

    #[test]
    fn rust_holders_are_counted_until_dropped() {
        let buffer = NativeBuffer::zeroed(8);
        let region = buffer.held.region().clone();
        assert_eq!(region.rust_holders(), 1);
        drop(buffer);
        assert_eq!(region.rust_holders(), 0);
    }

    #[test]
    fn overlapping_borrows_clash_when_one_is_written() {
        let memory = [0u8; 64];
        let at = memory.as_ptr();
        let ((), _) = framed(Kind::Call, Vec::new(), || {
            assert!(borrow(at, 32, false).is_ok());
            assert!(borrow(at, 16, false).is_ok());
            assert!(borrow(unsafe { at.add(32) }, 32, true).is_ok());
            assert!(borrow(unsafe { at.add(16) }, 1, true).is_err());
            assert!(borrow(unsafe { at.add(40) }, 4, false).is_err());
            assert!(borrow(at, 0, true).is_ok());
        });
        assert!(borrow(at, 64, true).is_ok());
    }
}
