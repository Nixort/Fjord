// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

// Fjord aarch64 boot shim (QEMU `virt`).
//
// The kernel carries a Linux arm64 Image header (see the arm64 booting
// protocol) so QEMU's `-kernel` loader treats the flat binary as a Linux
// kernel: it loads the image at RAM base + text_offset (0x4008_0000), enters at
// the first instruction, and -- crucially -- passes the flattened device-tree
// blob pointer in `x0`. A bare ELF does NOT get the DTB in `x0`, which is why
// the build objcopies the ELF to a raw binary before launch.
//
// Depending on the QEMU configuration the CPU may start in EL2 or EL1; this
// shim normalises to EL1, installs a minimal exception vector table, sets up a
// boot stack, clears `.bss`, then calls the Rust entry `rust_entry`, preserving
// the DTB pointer as the first argument.
//
// The integrated assembler defaults to AArch64 syntax; comments use `//`.

.section .text._start, "ax"
.global _start
_start:
    // ---- Linux arm64 Image header (Documentation/arm64/booting.rst) --------
    // Lets QEMU boot the flat binary via the Linux protocol and hand us the
    // DTB pointer in x0. code0 branches over the 64-byte header to _entry.
    b       _entry                  // code0 @0:  branch past the header
    .long   0                       // code1 @4:  reserved
    .quad   0x80000                 // text_offset @8:  load offset from RAM base
    .quad   __image_size            // image_size @16: bytes to reserve (incl .bss)
    .quad   0                       // flags @24: LE, 4 KiB pages, fixed placement
    .quad   0                       // res2 @32
    .quad   0                       // res3 @40
    .quad   0                       // res4 @48
    .ascii  "ARM\x64"               // magic @56: 0x644d5241
    .long   0                       // res5 @60: PE/COFF header offset (unused)

_entry:
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

    // Let EL1 drive the physical counter/timer without trapping to EL2.
    mov     x0, #0x3                // CNTHCTL_EL2.EL1PCTEN | EL1PCEN
    msr     cnthctl_el2, x0
    msr     cntvoff_el2, xzr        // identity virtual-counter offset

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
// EL1 exception vector table: 16 entries x 128 bytes, 2 KiB-aligned.
//
// The Current-EL synchronous slots dispatch through `el1_sync` (hull::arch::
// aarch64) which decodes ESR_EL1 and reports the fault; the Current-EL SPx IRQ
// slot dispatches through `el1_irq` (hull::gic). Lower-EL slots stay parked in
// `exc_halt` until user space lands (ROADMAP Phase 2).
// ---------------------------------------------------------------------------
.section .text.vectors, "ax"
.balign 0x800
.global __vectors
__vectors:
    .balign 0x80
    b       el1_sync            // Current EL, SP0:  Synchronous -> ESR decoder
    .balign 0x80
    b       exc_halt            // Current EL, SP0:  IRQ
    .balign 0x80
    b       exc_halt            // Current EL, SP0:  FIQ
    .balign 0x80
    b       exc_halt            // Current EL, SP0:  SError
    .balign 0x80
    b       el1_sync            // Current EL, SPx:  Synchronous -> ESR decoder
    .balign 0x80
    b       el1_irq             // Current EL, SPx:  IRQ -> GIC dispatcher
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
