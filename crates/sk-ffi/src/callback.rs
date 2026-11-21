//! C function pointers that call back into the host: a fixed pool of thunks written at build time.
//!
//! Every thunk jumps to one entry, which saves the argument registers and hands them to
//! [`dispatch`] with the thunk's number. Nothing is written into executable memory at run time.

use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};

use crate::{Signature, Type};

/// How many callbacks can be alive at once.
pub const CAPACITY: usize = 128;

/// The arguments, one normalized 64-bit slot each, to the return value's slot. It may run on any
/// thread the library calls from.
pub type Handler = Arc<dyn Fn(&[u64]) -> u64 + Send + Sync>;

struct Entry {
    signature: Signature,
    handler: Handler,
}

struct Pool {
    entries: Vec<Option<Entry>>,
    /// A released thunk goes to the back, so that a pointer a library kept by mistake reaches
    /// nothing for as long as possible instead of someone else's callback.
    free: VecDeque<usize>,
}

static POOL: Mutex<Option<Pool>> = Mutex::new(None);

fn with_pool<R>(f: impl FnOnce(&mut Pool) -> R) -> R {
    let mut guard = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let pool = guard.get_or_insert_with(|| Pool {
        entries: (0..CAPACITY).map(|_| None).collect(),
        free: (0..CAPACITY).collect(),
    });
    f(pool)
}

/// A thunk taken from the pool. Dropping it does not give the thunk back: [`Callback::release`] does.
#[derive(Debug)]
pub struct Callback {
    index: usize,
}

impl Callback {
    /// `None` when every thunk is taken, or when `signature` is variadic.
    pub fn new(signature: Signature, handler: Handler) -> Option<Callback> {
        if signature.fixed.is_some() || signature.args.contains(&Type::Void) {
            return None;
        }
        with_pool(|pool| {
            let index = pool.free.pop_front()?;
            pool.entries[index] = Some(Entry { signature, handler });
            Some(Callback { index })
        })
    }

    pub fn pointer(&self) -> *const () {
        (thunks::sk_ffi_thunks as *const () as usize + self.index * thunks::ENTRY) as *const ()
    }

    /// A call that arrives after this returns 0 and runs nothing; one already running finishes.
    pub fn release(self) {
        with_pool(|pool| {
            pool.entries[self.index] = None;
            pool.free.push_back(self.index);
        });
    }
}

/// How many thunks are taken.
pub fn live() -> usize {
    with_pool(|pool| CAPACITY - pool.free.len())
}

/// Where the entry saved what the caller passed.
struct Saved {
    gpr: *const u64,
    fpr: *const u64,
    #[cfg_attr(all(windows, target_arch = "x86_64"), allow(dead_code))]
    stack: *const u8,
}

/// The inverse of `abi::lay_out`.
#[cfg(all(windows, target_arch = "x86_64"))]
unsafe fn read(signature: &Signature, saved: &Saved) -> Vec<u64> {
    signature
        .args
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            let bits = if ty.is_float() && index < 4 {
                *saved.fpr.add(index)
            } else {
                *saved.gpr.add(index)
            };
            ty.normalize(bits)
        })
        .collect()
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
unsafe fn read(signature: &Signature, saved: &Saved) -> Vec<u64> {
    let int_registers = if cfg!(target_arch = "x86_64") { 6 } else { 8 };
    let packed = cfg!(all(target_arch = "aarch64", target_vendor = "apple"));
    let (mut ints, mut floats, mut offset) = (0, 0, 0usize);
    signature
        .args
        .iter()
        .map(|ty| {
            let bits = if ty.is_float() && floats < 8 {
                floats += 1;
                *saved.fpr.add(floats - 1)
            } else if !ty.is_float() && ints < int_registers {
                ints += 1;
                *saved.gpr.add(ints - 1)
            } else {
                let size = if packed { ty.size() } else { 8 };
                offset = offset.next_multiple_of(size);
                let mut bytes = [0u8; 8];
                std::ptr::copy_nonoverlapping(saved.stack.add(offset), bytes.as_mut_ptr(), size);
                offset += size;
                u64::from_le_bytes(bytes)
            };
            ty.normalize(bits)
        })
        .collect()
}

unsafe fn run(index: usize, saved: Saved, ret: *mut u64) {
    let entry = with_pool(|pool| {
        pool.entries
            .get(index)
            .and_then(Option::as_ref)
            .map(|entry| (entry.signature.clone(), entry.handler.clone()))
    });
    let value = match entry {
        Some((signature, handler)) => {
            let args = read(&signature, &saved);
            // A panic must not unwind into the library's frames.
            let value = catch_unwind(AssertUnwindSafe(|| handler(&args))).unwrap_or(0);
            signature.returns.normalize(value)
        }
        None => 0,
    };
    *ret = value;
    *ret.add(1) = value;
}

#[cfg(all(windows, target_arch = "x86_64"))]
unsafe extern "C" fn dispatch(index: usize, slots: *const u64, fpr: *const u64, ret: *mut u64) {
    let saved = Saved {
        gpr: slots,
        fpr,
        stack: std::ptr::null(),
    };
    run(index, saved, ret);
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
unsafe extern "C" fn dispatch(
    index: usize,
    gpr: *const u64,
    fpr: *const u64,
    stack: *const u8,
    ret: *mut u64,
) {
    run(index, Saved { gpr, fpr, stack }, ret);
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod thunks {
    use std::arch::naked_asm;

    pub const ENTRY: usize = 5;

    /// The four register arguments go to their home slots, which sit right below the stack
    /// arguments: together they are the arguments by position.
    #[unsafe(naked)]
    pub unsafe extern "C" fn sk_ffi_thunks() {
        naked_asm!(
            "5:",
            ".rept 128",
            "call 6f",
            ".endr",
            "6:",
            "pop r11",
            "push rbp",
            "mov rbp, rsp",
            "sub rsp, 96",
            "mov [rbp + 16], rcx",
            "mov [rbp + 24], rdx",
            "mov [rbp + 32], r8",
            "mov [rbp + 40], r9",
            "movq qword ptr [rbp - 32], xmm0",
            "movq qword ptr [rbp - 24], xmm1",
            "movq qword ptr [rbp - 16], xmm2",
            "movq qword ptr [rbp - 8], xmm3",
            "lea rax, [rip + 5b]",
            "sub r11, rax",
            "mov rax, r11",
            "xor edx, edx",
            "mov ecx, 5",
            "div rcx",
            "lea rcx, [rax - 1]",
            "lea rdx, [rbp + 16]",
            "lea r8, [rbp - 32]",
            "lea r9, [rbp - 48]",
            "call {dispatch}",
            "mov rax, [rbp - 48]",
            "movq xmm0, qword ptr [rbp - 40]",
            "mov rsp, rbp",
            "pop rbp",
            "ret",
            dispatch = sym super::dispatch,
        )
    }
}

#[cfg(all(unix, target_arch = "x86_64"))]
mod thunks {
    use std::arch::naked_asm;

    pub const ENTRY: usize = 5;

    #[unsafe(naked)]
    pub unsafe extern "C" fn sk_ffi_thunks() {
        naked_asm!(
            "5:",
            ".rept 128",
            "call 6f",
            ".endr",
            "6:",
            "pop r11",
            "push rbp",
            "mov rbp, rsp",
            "sub rsp, 128",
            "mov [rbp - 48], rdi",
            "mov [rbp - 40], rsi",
            "mov [rbp - 32], rdx",
            "mov [rbp - 24], rcx",
            "mov [rbp - 16], r8",
            "mov [rbp - 8], r9",
            "movq qword ptr [rbp - 112], xmm0",
            "movq qword ptr [rbp - 104], xmm1",
            "movq qword ptr [rbp - 96], xmm2",
            "movq qword ptr [rbp - 88], xmm3",
            "movq qword ptr [rbp - 80], xmm4",
            "movq qword ptr [rbp - 72], xmm5",
            "movq qword ptr [rbp - 64], xmm6",
            "movq qword ptr [rbp - 56], xmm7",
            "lea rax, [rip + 5b]",
            "sub r11, rax",
            "mov rax, r11",
            "xor edx, edx",
            "mov ecx, 5",
            "div rcx",
            "lea rdi, [rax - 1]",
            "lea rsi, [rbp - 48]",
            "lea rdx, [rbp - 112]",
            "lea rcx, [rbp + 16]",
            "lea r8, [rbp - 128]",
            "call {dispatch}",
            "mov rax, [rbp - 128]",
            "movq xmm0, qword ptr [rbp - 120]",
            "mov rsp, rbp",
            "pop rbp",
            "ret",
            dispatch = sym super::dispatch,
        )
    }
}

#[cfg(target_arch = "aarch64")]
mod thunks {
    use std::arch::naked_asm;

    pub const ENTRY: usize = 8;

    #[unsafe(naked)]
    pub unsafe extern "C" fn sk_ffi_thunks() {
        naked_asm!(
            "5:",
            ".rept 128",
            "1:",
            "adr x17, 1b",
            "b 6f",
            ".endr",
            "6:",
            "stp x29, x30, [sp, #-176]!",
            "mov x29, sp",
            "stp x0, x1, [sp, #16]",
            "stp x2, x3, [sp, #32]",
            "stp x4, x5, [sp, #48]",
            "stp x6, x7, [sp, #64]",
            "stp d0, d1, [sp, #80]",
            "stp d2, d3, [sp, #96]",
            "stp d4, d5, [sp, #112]",
            "stp d6, d7, [sp, #128]",
            "adr x16, 5b",
            "sub x0, x17, x16",
            "lsr x0, x0, #3",
            "add x1, sp, #16",
            "add x2, sp, #80",
            "add x3, sp, #176",
            "add x4, sp, #144",
            "bl {dispatch}",
            "ldr x0, [sp, #144]",
            "ldr d0, [sp, #152]",
            "ldp x29, x30, [sp], #176",
            "ret",
            dispatch = sym super::dispatch,
        )
    }
}
