//! The invoke stubs. Each takes a [`Frame`], copies its stack area below the stack pointer, loads
//! every argument register, calls, and stores both return registers.
//!
//! Frame offsets: function 0, stack 8, stack_len 16, gpr 24, fpr 88, ret_gpr 152 and 160,
//! ret_fpr 168 to 192, indirect 200.

use crate::abi::Frame;
use std::arch::naked_asm;

#[cfg(all(windows, target_arch = "x86_64"))]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn sk_ffi_invoke(frame: *mut Frame) {
    naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "push rbx",
        "sub rsp, 8",
        "mov rbx, rcx",
        "mov rax, [rbx + 16]",
        "sub rsp, rax",
        "mov r10, [rbx + 8]",
        "xor r11, r11",
        "2:",
        "cmp r11, rax",
        "jae 3f",
        "mov rdx, [r10 + r11]",
        "mov [rsp + r11], rdx",
        "add r11, 8",
        "jmp 2b",
        "3:",
        "mov rcx, [rbx + 24]",
        "mov rdx, [rbx + 32]",
        "mov r8, [rbx + 40]",
        "mov r9, [rbx + 48]",
        "movq xmm0, qword ptr [rbx + 88]",
        "movq xmm1, qword ptr [rbx + 96]",
        "movq xmm2, qword ptr [rbx + 104]",
        "movq xmm3, qword ptr [rbx + 112]",
        "call qword ptr [rbx]",
        "mov [rbx + 152], rax",
        "movq qword ptr [rbx + 168], xmm0",
        "lea rsp, [rbp - 8]",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}

#[cfg(all(unix, target_arch = "x86_64"))]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn sk_ffi_invoke(frame: *mut Frame) {
    naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "push rbx",
        "sub rsp, 8",
        "mov rbx, rdi",
        "mov rax, [rbx + 16]",
        "sub rsp, rax",
        "mov r10, [rbx + 8]",
        "xor r11, r11",
        "2:",
        "cmp r11, rax",
        "jae 3f",
        "mov rdx, [r10 + r11]",
        "mov [rsp + r11], rdx",
        "add r11, 8",
        "jmp 2b",
        "3:",
        "mov rdi, [rbx + 24]",
        "mov rsi, [rbx + 32]",
        "mov rdx, [rbx + 40]",
        "mov rcx, [rbx + 48]",
        "mov r8, [rbx + 56]",
        "mov r9, [rbx + 64]",
        "movq xmm0, qword ptr [rbx + 88]",
        "movq xmm1, qword ptr [rbx + 96]",
        "movq xmm2, qword ptr [rbx + 104]",
        "movq xmm3, qword ptr [rbx + 112]",
        "movq xmm4, qword ptr [rbx + 120]",
        "movq xmm5, qword ptr [rbx + 128]",
        "movq xmm6, qword ptr [rbx + 136]",
        "movq xmm7, qword ptr [rbx + 144]",
        // A variadic callee reads in al an upper bound of the vector registers used.
        "mov eax, 8",
        "call qword ptr [rbx]",
        "mov [rbx + 152], rax",
        "mov [rbx + 160], rdx",
        "movq qword ptr [rbx + 168], xmm0",
        "movq qword ptr [rbx + 176], xmm1",
        "lea rsp, [rbp - 8]",
        "pop rbx",
        "pop rbp",
        "ret",
    )
}

#[cfg(target_arch = "aarch64")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn sk_ffi_invoke(frame: *mut Frame) {
    naked_asm!(
        "stp x29, x30, [sp, #-32]!",
        "mov x29, sp",
        "str x19, [sp, #16]",
        "mov x19, x0",
        "ldr x9, [x19, #16]",
        "sub sp, sp, x9",
        "ldr x10, [x19, #8]",
        "mov x11, #0",
        "2:",
        "cmp x11, x9",
        "b.hs 3f",
        "ldr x12, [x10, x11]",
        "str x12, [sp, x11]",
        "add x11, x11, #8",
        "b 2b",
        "3:",
        "ldp x0, x1, [x19, #24]",
        "ldp x2, x3, [x19, #40]",
        "ldp x4, x5, [x19, #56]",
        "ldp x6, x7, [x19, #72]",
        "ldp d0, d1, [x19, #88]",
        "ldp d2, d3, [x19, #104]",
        "ldp d4, d5, [x19, #120]",
        "ldp d6, d7, [x19, #136]",
        "ldr x8, [x19, #200]",
        "ldr x9, [x19]",
        "blr x9",
        "stp x0, x1, [x19, #152]",
        "stp d0, d1, [x19, #168]",
        "stp d2, d3, [x19, #184]",
        "mov sp, x29",
        "ldr x19, [sp, #16]",
        "ldp x29, x30, [sp], #32",
        "ret",
    )
}
