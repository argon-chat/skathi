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

/// A struct passed or returned by value: its size, its alignment and where each scalar in it lies,
/// nested structs and arrays flattened. Fields sit at multiples of their size, as a C compiler puts them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructLayout {
    pub size: usize,
    pub align: usize,
    pub fields: Vec<(Type, usize)>,
}

/// An argument of [`call_with`].
#[derive(Clone, Copy)]
pub enum Argument<'a> {
    /// An integer's value or a float's bits.
    Scalar(Type, u64),
    /// The bytes of a struct, `size` of them.
    Struct(&'a StructLayout, *const u8),
}

/// What [`call_with`] returns.
#[derive(Clone, Copy)]
pub enum Returning<'a> {
    Scalar(Type),
    /// Where to write the struct, room for `size` bytes.
    Struct(&'a StructLayout, *mut u8),
}

/// Calls `function` with `args`, one 64-bit slot per argument: an integer's value, a float's bits.
///
/// # Safety
/// `function` is a C function of `signature`, and every pointer among `args` is valid for it.
pub unsafe fn call(function: *const (), signature: &Signature, args: &[u64]) -> Outcome {
    assert_eq!(args.len(), signature.args.len());
    assert!(!signature.args.contains(&Type::Void));
    let values = signature
        .args
        .iter()
        .zip(args)
        .map(|(ty, bits)| abi::Value::Scalar(*ty, ty.normalize(*bits)));
    let returns = abi::Returns::Scalar(signature.returns);
    invoke(
        function,
        abi::lay_out(values, returns, signature.fixed),
        returns,
    )
}

/// [`call`] for a function that takes or returns structs by value. The value of the outcome is 0
/// when a struct comes back.
///
/// # Safety
/// As [`call`]; and every struct argument points at the bytes of its layout, a struct return at room for them.
pub unsafe fn call_with(
    function: *const (),
    args: &[Argument<'_>],
    returning: Returning<'_>,
    fixed: Option<usize>,
) -> Outcome {
    let values = args.iter().map(|arg| match *arg {
        Argument::Scalar(ty, bits) => {
            assert!(ty != Type::Void);
            abi::Value::Scalar(ty, ty.normalize(bits))
        }
        Argument::Struct(layout, bytes) => abi::Value::Struct(layout, bytes),
    });
    let returns = match returning {
        Returning::Scalar(ty) => abi::Returns::Scalar(ty),
        Returning::Struct(layout, to) => abi::Returns::Struct(layout, to),
    };
    invoke(function, abi::lay_out(values, returns, fixed), returns)
}

unsafe fn invoke(function: *const (), mut frame: abi::Frame, returns: abi::Returns<'_>) -> Outcome {
    frame.function = function as usize;
    frame.seal();
    invoke::sk_ffi_invoke(&mut frame);
    let os_error = library::last_os_error();
    let value = match returns {
        abi::Returns::Scalar(ty) => {
            let raw = if ty.is_float() {
                frame.ret_fpr[0]
            } else {
                frame.ret_gpr[0]
            };
            ty.normalize(raw)
        }
        abi::Returns::Struct(layout, to) => {
            abi::collect(&frame, layout, to);
            0
        }
    };
    Outcome { value, os_error }
}

#[cfg(test)]
mod tests;
