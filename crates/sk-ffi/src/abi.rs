//! Where each argument goes: one layout per calling convention.

use crate::{Signature, Type};

/// Read by the invoke stubs at fixed offsets.
#[repr(C)]
pub(crate) struct Frame {
    pub function: usize,
    stack: *const u8,
    stack_len: usize,
    gpr: [u64; 8],
    fpr: [u64; 8],
    pub ret_gpr: u64,
    pub ret_fpr: u64,
    /// The stack area of most calls; `u64`s so that it is aligned for any argument.
    inline: [u64; INLINE],
    /// The stack area of a call with more than `inline` holds.
    spill: Vec<u64>,
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

struct Layout {
    gpr: [u64; 8],
    fpr: [u64; 8],
    inline: [u64; INLINE],
    spill: Vec<u64>,
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

    fn push(&mut self, bits: u64, size: usize, align: usize) {
        let at = self.len.next_multiple_of(align);
        self.reserve(at + size);
        let area: &mut [u64] = if self.spill.is_empty() {
            &mut self.inline
        } else {
            &mut self.spill
        };
        // SAFETY: `reserve` made the area hold `at + size` bytes.
        unsafe {
            let to = area.as_mut_ptr().cast::<u8>().add(at);
            std::ptr::copy_nonoverlapping(bits.to_le_bytes().as_ptr(), to, size);
        }
    }

    fn slot(&mut self, bits: u64) {
        self.push(bits, 8, 8);
    }

    fn finish(mut self) -> Frame {
        self.reserve(self.len.next_multiple_of(16));
        Frame {
            function: 0,
            stack: std::ptr::null(),
            stack_len: self.len,
            gpr: self.gpr,
            fpr: self.fpr,
            ret_gpr: 0,
            ret_fpr: 0,
            inline: self.inline,
            spill: self.spill,
        }
    }
}

pub(crate) fn lay_out(signature: &Signature, args: &[u64]) -> Frame {
    let mut layout = Layout {
        gpr: [0; 8],
        fpr: [0; 8],
        inline: [0; INLINE],
        spill: Vec::new(),
        len: 0,
    };
    let values = signature
        .args
        .iter()
        .zip(args)
        .map(|(ty, bits)| (*ty, ty.normalize(*bits)));
    place(&mut layout, signature, values);
    layout.finish()
}

/// Win64: an argument's position alone picks its register, and a float also travels in the
/// integer register of its position, which is what a variadic callee reads.
#[cfg(all(windows, target_arch = "x86_64"))]
fn place(layout: &mut Layout, _: &Signature, values: impl Iterator<Item = (Type, u64)>) {
    layout.reserve(32);
    for (index, (_, bits)) in values.enumerate() {
        if index < 4 {
            layout.gpr[index] = bits;
            layout.fpr[index] = bits;
        } else {
            layout.slot(bits);
        }
    }
}

/// System V x86-64: six integer and eight vector registers, counted apart.
#[cfg(all(unix, target_arch = "x86_64"))]
fn place(layout: &mut Layout, _: &Signature, values: impl Iterator<Item = (Type, u64)>) {
    let (mut ints, mut floats) = (0, 0);
    for (ty, bits) in values {
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
}

/// AAPCS64: eight integer and eight vector registers. On the stack an argument takes eight
/// bytes, except on Apple platforms, where it takes its own size, and where the unnamed
/// arguments of a variadic function all go on the stack in eight-byte slots.
#[cfg(target_arch = "aarch64")]
fn place(layout: &mut Layout, signature: &Signature, values: impl Iterator<Item = (Type, u64)>) {
    let apple = cfg!(target_vendor = "apple");
    let fixed = signature.fixed.unwrap_or(usize::MAX);
    let (mut ints, mut floats) = (0, 0);
    for (index, (ty, bits)) in values.enumerate() {
        let unnamed = index >= fixed;
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
