//! Calls a C function from its address and a signature known only at run time.
//!
//! Nothing is generated at run time: one invoke stub per ABI, written at build time, loads the
//! registers and the stack from a [`Frame`] that [`call`] lays out.

mod abi;
mod callback;
mod crash;
mod invoke;
mod library;

pub use callback::{live as live_callbacks, Callback, Handler, CAPACITY as CALLBACK_CAPACITY};
pub use crash::in_call;
pub use library::{Library, LoadError};

/// A C type as a signature names it. Buffers and strings are pointers by the time they get here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    Void,
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Isize,
    Usize,
    F32,
    F64,
    Pointer,
}

impl Type {
    pub fn size(self) -> usize {
        match self {
            Type::Void => 0,
            Type::Bool | Type::I8 | Type::U8 => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::I64 | Type::U64 | Type::F64 => 8,
            Type::Isize | Type::Usize | Type::Pointer => std::mem::size_of::<usize>(),
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Type::F32 | Type::F64)
    }

    fn signed(self) -> bool {
        matches!(
            self,
            Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Isize
        )
    }

    /// `bits` as the 64-bit slot that carries a value of this type: an integer extended from its
    /// own width, so nothing above it reaches the callee or the caller.
    pub fn normalize(self, bits: u64) -> u64 {
        match self {
            Type::Void => 0,
            Type::Bool => (bits & 0xff != 0) as u64,
            Type::F32 => bits & 0xffff_ffff,
            Type::F64 => bits,
            _ => {
                let unused = 64 - 8 * self.size() as u32;
                if unused == 0 {
                    bits
                } else if self.signed() {
                    (((bits << unused) as i64) >> unused) as u64
                } else {
                    (bits << unused) >> unused
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub args: Vec<Type>,
    pub returns: Type,
    /// For a variadic function, how many of `args` are its named parameters.
    pub fixed: Option<usize>,
}

/// What a call left behind besides its value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub value: u64,
    /// `errno` on POSIX, `GetLastError()` on Windows, read on the calling thread right after the call.
    pub os_error: i32,
}

/// Calls `function` with `args`, one 64-bit slot per argument: an integer's value, a float's bits.
///
/// # Safety
/// `function` is a C function of `signature`, and every pointer among `args` is valid for it.
pub unsafe fn call(function: *const (), signature: &Signature, args: &[u64]) -> Outcome {
    assert_eq!(args.len(), signature.args.len());
    assert!(!signature.args.contains(&Type::Void));
    let mut frame = abi::lay_out(signature, args);
    frame.function = function as usize;
    frame.seal();
    invoke::sk_ffi_invoke(&mut frame);
    let os_error = library::last_os_error();
    let raw = if signature.returns.is_float() {
        frame.ret_fpr
    } else {
        frame.ret_gpr
    };
    Outcome {
        value: signature.returns.normalize(raw),
        os_error,
    }
}

#[cfg(test)]
mod tests;
