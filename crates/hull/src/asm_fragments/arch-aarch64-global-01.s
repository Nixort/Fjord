
.section .text.vectors, "ax"
.global fjord_aarch64_user_run
fjord_aarch64_user_run:
    stp     x19, x20, [sp, #-16]!
    stp     x21, x22, [sp, #-16]!
    stp     x23, x24, [sp, #-16]!
    stp     x25, x26, [sp, #-16]!
    stp     x27, x28, [sp, #-16]!
    stp     x29, x30, [sp, #-16]!
    mrs     x9, daif
    stp     x9, x9, [sp, #-16]!
    adrp    x10, FJORD_AARCH64_KERNEL_SP
    add     x10, x10, #:lo12:FJORD_AARCH64_KERNEL_SP
    mov     x11, sp
    str     x11, [x10]
    adrp    x10, FJORD_AARCH64_CURRENT_FRAME
    add     x10, x10, #:lo12:FJORD_AARCH64_CURRENT_FRAME
    str     x0, [x10]
    // Program the EL0 return state from the frame.
    ldr     x9, [x0, #0xF8]
    msr     sp_el0, x9
    ldr     x9, [x0, #0x100]
    msr     elr_el1, x9
    ldr     x9, [x0, #0x108]
    msr     spsr_el1, x9
    // Load the user GPRs (x0 last, since it is the base pointer).
    ldp     x1, x2,   [x0, #0x08]
    ldp     x3, x4,   [x0, #0x18]
    ldp     x5, x6,   [x0, #0x28]
    ldp     x7, x8,   [x0, #0x38]
    ldp     x9, x10,  [x0, #0x48]
    ldp     x11, x12, [x0, #0x58]
    ldp     x13, x14, [x0, #0x68]
    ldp     x15, x16, [x0, #0x78]
    ldp     x17, x18, [x0, #0x88]
    ldp     x19, x20, [x0, #0x98]
    ldp     x21, x22, [x0, #0xA8]
    ldp     x23, x24, [x0, #0xB8]
    ldp     x25, x26, [x0, #0xC8]
    ldp     x27, x28, [x0, #0xD8]
    ldp     x29, x30, [x0, #0xE8]
    ldr     x0, [x0, #0x00]
    eret

.global el0_sync
el0_sync:
    // Running at EL1 on the kernel stack. Free x9/x10 as scratch (saving the
    // user values), then dispatch on the exception class.
    stp     x9, x10, [sp, #-16]!
    mrs     x9, esr_el1
    lsr     x10, x9, #26
    and     x10, x10, #0x3f
    cmp     x10, #0x15                  // EC == SVC (AArch64)?
    b.ne    2f
    // Save the full EL0 register state into the current frame.
    adrp    x9, FJORD_AARCH64_CURRENT_FRAME
    add     x9, x9, #:lo12:FJORD_AARCH64_CURRENT_FRAME
    ldr     x9, [x9]
    stp     x0, x1,   [x9, #0x00]
    stp     x2, x3,   [x9, #0x10]
    stp     x4, x5,   [x9, #0x20]
    stp     x6, x7,   [x9, #0x30]
    str     x8,       [x9, #0x40]
    str     x11,      [x9, #0x58]
    stp     x12, x13, [x9, #0x60]
    stp     x14, x15, [x9, #0x70]
    stp     x16, x17, [x9, #0x80]
    stp     x18, x19, [x9, #0x90]
    stp     x20, x21, [x9, #0xA0]
    stp     x22, x23, [x9, #0xB0]
    stp     x24, x25, [x9, #0xC0]
    stp     x26, x27, [x9, #0xD0]
    stp     x28, x29, [x9, #0xE0]
    str     x30,      [x9, #0xF0]
    // Recover the user x9/x10 stashed on the kernel stack and store them.
    ldp     x0, x1, [sp]
    str     x0, [x9, #0x48]
    str     x1, [x9, #0x50]
    // System registers: SP_EL0, ELR_EL1 (PC after svc), SPSR_EL1.
    mrs     x0, sp_el0
    str     x0, [x9, #0xF8]
    mrs     x0, elr_el1
    str     x0, [x9, #0x100]
    mrs     x0, spsr_el1
    str     x0, [x9, #0x108]
    add     sp, sp, #16                 // drop the scratch slot
    // Restore kernel callee-saved state and unwind to the user_run caller.
    adrp    x9, FJORD_AARCH64_KERNEL_SP
    add     x9, x9, #:lo12:FJORD_AARCH64_KERNEL_SP
    ldr     x10, [x9]
    mov     sp, x10
    ldp     x9, x10, [sp], #16
    msr     daif, x9
    ldp     x29, x30, [sp], #16
    ldp     x27, x28, [sp], #16
    ldp     x25, x26, [sp], #16
    ldp     x23, x24, [sp], #16
    ldp     x21, x22, [sp], #16
    ldp     x19, x20, [sp], #16
    ret
2:
    add     sp, sp, #16                 // drop the scratch slot before the fatal path
    b       el1_sync
