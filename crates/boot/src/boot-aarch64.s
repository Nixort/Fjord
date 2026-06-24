// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

// Fjord aarch64 boot shim (QEMU `virt`).
//
// QEMU loads this freestanding ELF via `-kernel` at its physical link address
// (0x4008_0000) and enters at `_start`. Depending on the QEMU configuration the
// CPU may start in EL2 or EL1; this shim normalises to EL1, installs a minimal
// exception vector table, sets up a boot stack, clears `.bss`, then calls the
// Rust entry `rust_entry`. The flattened device-tree pointer QEMU passes in
// `x0` is preserved as the first argument.
//
// The integrated assembler defaults to AArch64 syntax; comments use `//`.

.section .text._start, "ax"
.global _start
_start:
    // Preserve the DTB pointer (x0) across early bring-up.
    mov     x19, x0

    // Only the primary CPU (Aff0 == 0) boots; secondaries park.
    mrs     x1, mpidr_el1
    and     x1, x1, #0xff
    cbnz    x1, park_cpu

    // Mask all interrupts (D, A, I, F) during bring-up.
    msr     daifset, #0xf

    // Normalise to EL1. If we entered at EL2, configure EL1 and `eret` down;
    // otherwise assume EL1 and continue.
    mrs     x0, currentel
    lsr     x0, x0, #2
    cmp     x0, #2
    b.ne    el1_entry

    // EL2 -> EL1: make EL1 execute AArch64, hand it a masked SPSR, and return
    // into `el1_entry` running at EL1h.
    mrs     x0, hcr_el2
    orr     x0, x0, #(1 << 31)      // HCR_EL2.RW = 1 (EL1 is AArch64)
    msr     hcr_el2, x0
    mov     x0, #0x3c5              // SPSR: EL1h, DAIF masked
    msr     spsr_el2, x0
    adr     x0, el1_entry
    msr     elr_el2, x0
    eret

el1_entry:
    // Boot stack.
    adrp    x0, __boot_stack_top
    add     x0, x0, #:lo12:__boot_stack_top
    mov     sp, x0

    // Install the EL1 exception vectors.
    adrp    x0, __vectors
    add     x0, x0, #:lo12:__vectors
    msr     vbar_el1, x0
    isb

    // Zero `.bss` over [__bss_start, __bss_end) (8 bytes at a time; the linker
    // 16-byte-aligns both bounds).
    adrp    x0, __bss_start
    add     x0, x0, #:lo12:__bss_start
    adrp    x1, __bss_end
    add     x1, x1, #:lo12:__bss_end
1:
    cmp     x0, x1
    b.hs    2f
    str     xzr, [x0], #8
    b       1b
2:
    // First argument: the preserved DTB pointer. Enter Rust; never returns.
    mov     x0, x19
    bl      rust_entry

    // `rust_entry` must not return; halt defensively if it ever does.
park_cpu:
    wfe
    b       park_cpu

// ---------------------------------------------------------------------------
// Minimal EL1 exception vector table: 16 entries x 128 bytes, 2 KiB-aligned.
//
// During early bring-up interrupts are masked and no exceptions are expected,
// so every vector funnels into a halt loop rather than a dispatcher. Real
// handlers arrive with the GIC + generic-timer slice (ROADMAP Phase 1).
// ---------------------------------------------------------------------------
.section .text.vectors, "ax"
.balign 0x800
.global __vectors
__vectors:
    .balign 0x80
    b       exc_halt            // Current EL, SP0:  Synchronous
    .balign 0x80
    b       exc_halt            // Current EL, SP0:  IRQ
    .balign 0x80
    b       exc_halt            // Current EL, SP0:  FIQ
    .balign 0x80
    b       exc_halt            // Current EL, SP0:  SError
    .balign 0x80
    b       exc_halt            // Current EL, SPx:  Synchronous
    .balign 0x80
    b       exc_halt            // Current EL, SPx:  IRQ
    .balign 0x80
    b       exc_halt            // Current EL, SPx:  FIQ
    .balign 0x80
    b       exc_halt            // Current EL, SPx:  SError
    .balign 0x80
    b       exc_halt            // Lower EL, AArch64: Synchronous
    .balign 0x80
    b       exc_halt            // Lower EL, AArch64: IRQ
    .balign 0x80
    b       exc_halt            // Lower EL, AArch64: FIQ
    .balign 0x80
    b       exc_halt            // Lower EL, AArch64: SError
    .balign 0x80
    b       exc_halt            // Lower EL, AArch32: Synchronous
    .balign 0x80
    b       exc_halt            // Lower EL, AArch32: IRQ
    .balign 0x80
    b       exc_halt            // Lower EL, AArch32: FIQ
    .balign 0x80
    b       exc_halt            // Lower EL, AArch32: SError

.balign 4
exc_halt:
    wfe
    b       exc_halt
