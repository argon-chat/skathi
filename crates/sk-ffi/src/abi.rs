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
    /// Keeps `stack` alive; `u64`s so that it is aligned for any argument.
    owner: Vec<u64>,
}

#[derive(Default)]
struct Layout {
    gpr: [u64; 8],
    fpr: [u64; 8],
    stack: Vec<u8>,
}

impl Layout {
    fn push(&mut self, bits: u64, size: usize, align: usize) {
        while !self.stack.len().is_multiple_of(align) {
            self.stack.push(0);
        }
        self.stack.extend_from_slice(&bits.to_le_bytes()[..size]);
    }

    fn slot(&mut self, bits: u64) {
        self.push(bits, 8, 8);
    }

    fn finish(mut self) -> Frame {
        while !self.stack.len().is_multiple_of(16) {
            self.stack.push(0);
        }
        let mut owner = vec![0u64; self.stack.len() / 8];
        for (word, bytes) in owner.iter_mut().zip(self.stack.chunks_exact(8)) {
            *word = u64::from_le_bytes(bytes.try_into().unwrap());
        }
        Frame {
            function: 0,
            stack: owner.as_ptr().cast(),
            stack_len: self.stack.len(),
            gpr: self.gpr,
            fpr: self.fpr,
            ret_gpr: 0,
            ret_fpr: 0,
            owner,
        }
    }
}

pub(crate) fn lay_out(signature: &Signature, args: &[u64]) -> Frame {
    let mut layout = Layout::default();
    let values = signature
        .args
        .iter()
        .zip(args)
        .map(|(ty, bits)| (*ty, ty.normalize(*bits)));
    place(&mut layout, signature, values);
    let frame = layout.finish();
    debug_assert_eq!(frame.owner.len() * 8, frame.stack_len);
    frame
}

/// Win64: an argument's position alone picks its register, and a float also travels in the
/// integer register of its position, which is what a variadic callee reads.
#[cfg(all(windows, target_arch = "x86_64"))]
fn place(layout: &mut Layout, _: &Signature, values: impl Iterator<Item = (Type, u64)>) {
    layout.stack.resize(32, 0);
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
