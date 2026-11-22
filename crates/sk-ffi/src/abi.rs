//! Where each argument goes: one layout per calling convention.

use crate::{StructLayout, Type};

/// Read by the invoke stubs at fixed offsets.
#[repr(C)]
pub(crate) struct Frame {
    pub function: usize,
    stack: *const u8,
    stack_len: usize,
    gpr: [u64; 8],
    fpr: [u64; 8],
    pub ret_gpr: [u64; 2],
    pub ret_fpr: [u64; 4],
    /// AAPCS64's indirect result register: where a callee writes a large struct it returns.
    indirect: u64,
    /// The stack area of most calls; `u64`s so that it is aligned for any argument.
    inline: [u64; INLINE],
    /// The stack area of a call with more than `inline` holds.
    spill: Vec<u64>,
    /// The copies of the structs passed by reference, which the callee may write to.
    _copies: Vec<Vec<u64>>,
}

const INLINE: usize = 16;

impl Frame {
    /// Points `stack` at the area. The frame must not move after this.
    pub fn seal(&mut self) {
        self.stack = if self.spill.is_empty() {
            self.inline.as_ptr().cast()
        } else {
            self.spill.as_ptr().cast()
        };
    }
}

/// An argument: a scalar's 64-bit slot, or a struct's layout and its bytes.
#[derive(Clone, Copy)]
pub(crate) enum Value<'a> {
    Scalar(#[cfg_attr(windows, allow(dead_code))] Type, u64),
    Struct(&'a StructLayout, *const u8),
}

/// What comes back: a scalar, or a struct written to the given bytes.
#[derive(Clone, Copy)]
pub(crate) enum Returns<'a> {
    Scalar(Type),
    Struct(&'a StructLayout, *mut u8),
}

struct Layout {
    gpr: [u64; 8],
    fpr: [u64; 8],
    indirect: u64,
    inline: [u64; INLINE],
    spill: Vec<u64>,
    copies: Vec<Vec<u64>>,
    /// Bytes of the stack area in use.
    len: usize,
}

impl Layout {
    fn reserve(&mut self, len: usize) {
        self.len = len;
        let words = len.div_ceil(8);
        if words > INLINE || !self.spill.is_empty() {
            if self.spill.is_empty() {
                self.spill.extend_from_slice(&self.inline);
            }
            if self.spill.len() < words {
                self.spill.resize(words, 0);
            }
        }
    }

    /// SAFETY: `from` is valid for `size` bytes.
    unsafe fn push_bytes(&mut self, from: *const u8, size: usize, align: usize) {
        let at = self.len.next_multiple_of(align);
        self.reserve(at + size);
        let area: &mut [u64] = if self.spill.is_empty() {
            &mut self.inline
        } else {
            &mut self.spill
        };
        // `reserve` made the area hold `at + size` bytes.
        let to = area.as_mut_ptr().cast::<u8>().add(at);
        std::ptr::copy_nonoverlapping(from, to, size);
    }

    fn push(&mut self, bits: u64, size: usize, align: usize) {
        // SAFETY: the eight bytes of `bits` hold `size`.
        unsafe { self.push_bytes(bits.to_le_bytes().as_ptr(), size, align) }
    }

    fn slot(&mut self, bits: u64) {
        self.push(bits, 8, 8);
    }

    /// A struct on the stack in whole eight-byte slots.
    #[cfg_attr(windows, allow(dead_code))]
    unsafe fn slots(&mut self, layout: &StructLayout, from: *const u8) {
        self.push_bytes(from, layout.size, layout.align.max(8));
        self.reserve(self.len.next_multiple_of(8));
    }

    /// A private copy of a struct the callee gets a pointer to.
    #[cfg_attr(all(unix, target_arch = "x86_64"), allow(dead_code))]
    unsafe fn copy(&mut self, layout: &StructLayout, from: *const u8) -> u64 {
        let mut copy = vec![0u64; layout.size.div_ceil(8).max(1)];
        std::ptr::copy_nonoverlapping(from, copy.as_mut_ptr().cast::<u8>(), layout.size);
        let at = copy.as_ptr() as u64;
        self.copies.push(copy);
        at
    }

    fn finish(mut self) -> Frame {
        self.reserve(self.len.next_multiple_of(16));
        Frame {
            function: 0,
            stack: std::ptr::null(),
            stack_len: self.len,
            gpr: self.gpr,
            fpr: self.fpr,
            ret_gpr: [0; 2],
            ret_fpr: [0; 4],
            indirect: self.indirect,
            inline: self.inline,
            spill: self.spill,
            _copies: self.copies,
        }
    }
}

/// Up to eight bytes of a struct from `at`, zero-extended.
unsafe fn word(from: *const u8, at: usize, size: usize) -> u64 {
    let mut bytes = [0u8; 8];
    let len = (size - at).min(8);
    std::ptr::copy_nonoverlapping(from.add(at), bytes.as_mut_ptr(), len);
    u64::from_le_bytes(bytes)
}

unsafe fn store(to: *mut u8, at: usize, size: usize, bits: u64) {
    let len = (size - at).min(8);
    std::ptr::copy_nonoverlapping(bits.to_le_bytes().as_ptr(), to.add(at), len);
}

/// SAFETY: every struct value points at `size` bytes of its layout, and a struct return at room for them.
pub(crate) unsafe fn lay_out<'a>(
    values: impl Iterator<Item = Value<'a>>,
    returns: Returns<'a>,
    fixed: Option<usize>,
) -> Frame {
    let mut layout = Layout {
        gpr: [0; 8],
        fpr: [0; 8],
        indirect: 0,
        inline: [0; INLINE],
        spill: Vec::new(),
        copies: Vec::new(),
        len: 0,
    };
    place(&mut layout, fixed, returns, values);
    layout.finish()
}

/// Win64 passes a struct of 1, 2, 4 or 8 bytes as an integer of that size, and any other by pointer.
#[cfg(all(windows, target_arch = "x86_64"))]
fn as_integer(layout: &StructLayout) -> bool {
    matches!(layout.size, 1 | 2 | 4 | 8)
}

/// Win64: an argument's position alone picks its register, and a float also travels in the
/// integer register of its position, which is what a variadic callee reads.
#[cfg(all(windows, target_arch = "x86_64"))]
unsafe fn place<'a>(
    layout: &mut Layout,
    _: Option<usize>,
    returns: Returns<'a>,
    values: impl Iterator<Item = Value<'a>>,
) {
    layout.reserve(32);
    let hidden = match returns {
        Returns::Struct(of, to) if !as_integer(of) => Some(Value::Scalar(Type::Pointer, to as u64)),
        _ => None,
    };
    for (index, value) in hidden.into_iter().chain(values).enumerate() {
        let bits = match value {
            Value::Scalar(_, bits) => bits,
            Value::Struct(of, from) if as_integer(of) => word(from, 0, of.size),
            Value::Struct(of, from) => layout.copy(of, from),
        };
        if index < 4 {
            layout.gpr[index] = bits;
            layout.fpr[index] = bits;
        } else {
            layout.slot(bits);
        }
    }
}

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) unsafe fn collect(frame: &Frame, of: &StructLayout, to: *mut u8) {
    if as_integer(of) {
        store(to, 0, of.size, frame.ret_gpr[0]);
    }
}

/// System V classifies each eight bytes of a struct of up to sixteen: in a vector register when
/// floats are all it holds, in an integer register otherwise. `None`: the struct goes in memory.
#[cfg(all(unix, target_arch = "x86_64"))]
fn eightbytes(layout: &StructLayout) -> Option<Vec<bool>> {
    if layout.size > 16 || layout.size == 0 {
        return None;
    }
    let mut float = vec![true; layout.size.div_ceil(8)];
    for (ty, offset) in &layout.fields {
        if offset % ty.size() != 0 {
            return None;
        }
        float[offset / 8] &= ty.is_float();
    }
    Some(float)
}

/// System V x86-64: six integer and eight vector registers, counted apart.
#[cfg(all(unix, target_arch = "x86_64"))]
unsafe fn place<'a>(
    layout: &mut Layout,
    _: Option<usize>,
    returns: Returns<'a>,
    values: impl Iterator<Item = Value<'a>>,
) {
    let (mut ints, mut floats) = (0, 0);
    if let Returns::Struct(of, to) = returns {
        if eightbytes(of).is_none() {
            layout.gpr[0] = to as u64;
            ints = 1;
        }
    }
    for value in values {
        match value {
            Value::Scalar(ty, bits) => {
                if ty.is_float() && floats < 8 {
                    layout.fpr[floats] = bits;
                    floats += 1;
                } else if !ty.is_float() && ints < 6 {
                    layout.gpr[ints] = bits;
                    ints += 1;
                } else {
                    layout.slot(bits);
                }
            }
            Value::Struct(of, from) => {
                let classes = eightbytes(of).filter(|classes| {
                    let vectors = classes.iter().filter(|float| **float).count();
                    floats + vectors <= 8 && ints + classes.len() - vectors <= 6
                });
                let Some(classes) = classes else {
                    layout.slots(of, from);
                    continue;
                };
                for (index, float) in classes.into_iter().enumerate() {
                    let bits = word(from, index * 8, of.size);
                    if float {
                        layout.fpr[floats] = bits;
                        floats += 1;
                    } else {
                        layout.gpr[ints] = bits;
                        ints += 1;
                    }
                }
            }
        }
    }
}

#[cfg(all(unix, target_arch = "x86_64"))]
pub(crate) unsafe fn collect(frame: &Frame, of: &StructLayout, to: *mut u8) {
    let Some(classes) = eightbytes(of) else {
        return;
    };
    let (mut ints, mut floats) = (0, 0);
    for (index, float) in classes.into_iter().enumerate() {
        let bits = if float {
            floats += 1;
            frame.ret_fpr[floats - 1]
        } else {
            ints += 1;
            frame.ret_gpr[ints - 1]
        };
        store(to, index * 8, of.size, bits);
    }
}

/// AAPCS64's homogeneous floating-point aggregate: one to four floats of one type, each in a
/// vector register of its own.
#[cfg(target_arch = "aarch64")]
fn homogeneous(layout: &StructLayout) -> Option<Type> {
    let (first, _) = *layout.fields.first()?;
    let alike = layout.fields.iter().all(|(ty, _)| *ty == first);
    (first.is_float() && alike && layout.fields.len() <= 4).then_some(first)
}

/// AAPCS64: eight integer and eight vector registers. On the stack an argument takes eight
/// bytes, except on Apple platforms, where it takes its own size, and where the unnamed
/// arguments of a variadic function all go on the stack in eight-byte slots.
#[cfg(target_arch = "aarch64")]
unsafe fn place<'a>(
    layout: &mut Layout,
    fixed: Option<usize>,
    returns: Returns<'a>,
    values: impl Iterator<Item = Value<'a>>,
) {
    let apple = cfg!(target_vendor = "apple");
    let fixed = fixed.unwrap_or(usize::MAX);
    if let Returns::Struct(of, to) = returns {
        if homogeneous(of).is_none() && of.size > 16 {
            layout.indirect = to as u64;
        }
    }
    let (mut ints, mut floats) = (0, 0);
    for (index, value) in values.enumerate() {
        let unnamed = index >= fixed;
        let (ty, bits) = match value {
            Value::Scalar(ty, bits) => (ty, bits),
            Value::Struct(of, from) => {
                if let Some(member) = homogeneous(of) {
                    if floats + of.fields.len() <= 8 {
                        for (_, offset) in &of.fields {
                            layout.fpr[floats] =
                                word(from, *offset, (offset + member.size()).min(of.size));
                            floats += 1;
                        }
                    } else {
                        floats = 8;
                        stacked(layout, of, from, apple);
                    }
                    continue;
                }
                if of.size > 16 {
                    (Type::Pointer, layout.copy(of, from))
                } else {
                    let words = of.size.div_ceil(8);
                    if ints + words <= 8 {
                        for word_index in 0..words {
                            layout.gpr[ints] = word(from, word_index * 8, of.size);
                            ints += 1;
                        }
                    } else {
                        ints = 8;
                        stacked(layout, of, from, apple);
                    }
                    continue;
                }
            }
        };
        if apple && unnamed {
            layout.slot(bits);
        } else if ty.is_float() && floats < 8 {
            layout.fpr[floats] = bits;
            floats += 1;
        } else if !ty.is_float() && ints < 8 {
            layout.gpr[ints] = bits;
            ints += 1;
        } else if apple {
            layout.push(bits, ty.size(), ty.size());
        } else {
            layout.slot(bits);
        }
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn stacked(layout: &mut Layout, of: &StructLayout, from: *const u8, apple: bool) {
    if apple {
        layout.push_bytes(from, of.size, of.align);
    } else {
        layout.slots(of, from);
    }
}

#[cfg(target_arch = "aarch64")]
pub(crate) unsafe fn collect(frame: &Frame, of: &StructLayout, to: *mut u8) {
    if let Some(member) = homogeneous(of) {
        for (index, (_, offset)) in of.fields.iter().enumerate() {
            store(
                to,
                *offset,
                (offset + member.size()).min(of.size),
                frame.ret_fpr[index],
            );
        }
    } else if of.size <= 16 {
        for index in 0..of.size.div_ceil(8) {
            store(to, index * 8, of.size, frame.ret_gpr[index]);
        }
    }
}
