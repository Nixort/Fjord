// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! x86_64 local APIC bring-up and a periodic timer interrupt.
//!
//! Phase 1 only needs a heartbeat: disable the legacy 8259 PICs, enable the
//! local APIC, point its timer at a dedicated IDT vector in periodic mode and
//! enable interrupts. The handler increments a monotonic tick counter, signals
//! EOI and returns with `iretq` (unlike the CPU-exception path, which halts).
//!
//! The APIC register window lives at the architectural physical base
//! `0xFEE0_0000`, well above the identity-mapped RAM window, so it is mapped on
//! demand as an uncacheable device page via [`crate::paging::map_mmio_page`].
//!
//! Routing IRQs to userspace driver capabilities is a Phase 2 concern.

use crate::mmu::FrameAllocator;
use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

// Local APIC register offsets (from the MMIO base).
const REG_ID: u32 = 0x20;
const REG_EOI: u32 = 0xB0;
const REG_SVR: u32 = 0xF0;
const REG_LVT_TIMER: u32 = 0x320;
const REG_TIMER_INIT: u32 = 0x380;
const REG_TIMER_DIV: u32 = 0x3E0;

/// Architectural physical base of the local APIC MMIO window.
const LAPIC_BASE_PHYS: u64 = 0xFEE0_0000;

// Spurious-interrupt vector register bits.
const SVR_APIC_ENABLE: u32 = 1 << 8;

// LVT bits.
const LVT_MASKED: u32 = 1 << 16;
const LVT_PERIODIC: u32 = 1 << 17;

// Timer divide configuration: divide-by-16 (SDM encoding 0b0011).
const TIMER_DIV_16: u32 = 0b0011;
// Initial count: tuned so QEMU emits a few ticks per second, not a flood.
const TIMER_INITIAL_COUNT: u32 = 0x0100_0000;

/// IDT vector used for the periodic LAPIC timer.
const TIMER_VECTOR: u8 = 0x40;
/// IDT vector used for spurious LAPIC interrupts.
const SPURIOUS_VECTOR: u8 = 0xFF;

// IA32_APIC_BASE MSR and its global-enable bit.
const IA32_APIC_BASE: u32 = 0x1B;
const APIC_GLOBAL_ENABLE: u64 = 1 << 11;

// Legacy 8259 PIC ports.
const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

/// Virtual address of the mapped LAPIC window (identity == physical), or 0.
static LAPIC_BASE: AtomicU64 = AtomicU64::new(0);
/// Monotonic count of periodic timer interrupts observed so far.
static TICKS: AtomicU64 = AtomicU64::new(0);
/// After this many ticks the demo masks the timer to spare the serial console.
const TICK_DEMO_LIMIT: u64 = 5;

unsafe extern "C" {
    fn fjord_timer_isr();
    fn fjord_spurious_isr();
}

/// Write a byte to an I/O port.
///
/// # Safety
/// `port` must be a valid, side-effect-safe I/O port.
unsafe fn outb(port: u16, value: u8) {
    // SAFETY: caller upholds port validity.
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value,
             options(nomem, nostack, preserves_flags));
    }
}

/// Read a 64-bit model-specific register.
///
/// # Safety
/// `msr` must be a readable MSR on this CPU.
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: caller upholds MSR validity.
    unsafe {
        asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi,
             options(nomem, nostack, preserves_flags));
    }
    ((hi as u64) << 32) | (lo as u64)
}

/// Write a 64-bit model-specific register.
///
/// # Safety
/// `msr` must be a writable MSR and `value` valid for it.
unsafe fn wrmsr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    // SAFETY: caller upholds MSR validity.
    unsafe {
        asm!("wrmsr", in("ecx") msr, in("eax") lo, in("edx") hi,
             options(nomem, nostack, preserves_flags));
    }
}

/// Write a local APIC register.
///
/// # Safety
/// The LAPIC window must already be mapped (`LAPIC_BASE` set) and `offset` a
/// valid register offset.
unsafe fn lapic_write(offset: u32, value: u32) {
    let base = LAPIC_BASE.load(Ordering::Relaxed);
    // SAFETY: `base + offset` is inside the mapped MMIO page.
    unsafe { core::ptr::write_volatile((base + offset as u64) as *mut u32, value); }
}

/// Read a local APIC register.
///
/// # Safety
/// See [`lapic_write`].
unsafe fn lapic_read(offset: u32) -> u32 {
    let base = LAPIC_BASE.load(Ordering::Relaxed);
    // SAFETY: `base + offset` is inside the mapped MMIO page.
    unsafe { core::ptr::read_volatile((base + offset as u64) as *const u32) }
}

/// Remap the legacy 8259 PICs off the exception range and mask every line.
///
/// # Safety
/// Talks directly to the PIC I/O ports; call once during early boot.
unsafe fn disable_8259() {
    // SAFETY: standard ICW1..ICW4 init then full mask on the fixed PIC ports.
    unsafe {
        outb(PIC1_CMD, 0x11); // ICW1: begin init, expect ICW4
        outb(PIC2_CMD, 0x11);
        outb(PIC1_DATA, 0x20); // ICW2: master vectors 0x20..0x27
        outb(PIC2_DATA, 0x28); // ICW2: slave vectors 0x28..0x2F
        outb(PIC1_DATA, 0x04); // ICW3: slave on IRQ2
        outb(PIC2_DATA, 0x02); // ICW3: slave identity
        outb(PIC1_DATA, 0x01); // ICW4: 8086 mode
        outb(PIC2_DATA, 0x01);
        outb(PIC1_DATA, 0xFF); // mask all on the master
        outb(PIC2_DATA, 0xFF); // mask all on the slave
    }
}

/// Bring up the local APIC and start a periodic timer interrupt.
///
/// `kernel_pml4` must be the live kernel PML4 (the value loaded into CR3 by
/// [`crate::paging::init_kernel_address_space`]). Returns `true` once the timer
/// is armed and interrupts are enabled, or `false` if the APIC MMIO page could
/// not be mapped.
pub fn init_timer(kernel_pml4: u64, alloc: &mut FrameAllocator) -> bool {
    let base = match crate::paging::map_mmio_page(kernel_pml4, LAPIC_BASE_PHYS, alloc) {
        Some(va) => va,
        None => return false,
    };
    LAPIC_BASE.store(base, Ordering::Relaxed);

    // SAFETY: single-core early boot; the LAPIC window is mapped and the IDT is
    // loaded. We program the APIC, install gates and only then enable IRQs.
    unsafe {
        disable_8259();

        // Global-enable the APIC, preserving its base-address and BSP bits.
        let apic_base = rdmsr(IA32_APIC_BASE);
        wrmsr(IA32_APIC_BASE, apic_base | APIC_GLOBAL_ENABLE);

        // Software-enable the APIC and route spurious interrupts to a safe gate.
        crate::arch::x86_64::set_irq_gate(SPURIOUS_VECTOR, fjord_spurious_isr);
        lapic_write(REG_SVR, SVR_APIC_ENABLE | SPURIOUS_VECTOR as u32);

        // Install the timer gate, then program a periodic tick.
        crate::arch::x86_64::set_irq_gate(TIMER_VECTOR, fjord_timer_isr);
        lapic_write(REG_TIMER_DIV, TIMER_DIV_16);
        lapic_write(REG_LVT_TIMER, LVT_PERIODIC | TIMER_VECTOR as u32);
        lapic_write(REG_TIMER_INIT, TIMER_INITIAL_COUNT);

        let id = lapic_read(REG_ID) >> 24;
        crate::kprintln!("hull: local APIC #{id} enabled @ {base:#x}; legacy PIC masked");

        // Let the timer fire.
        asm!("sti", options(nomem, nostack));
    }
    true
}

/// Number of periodic timer interrupts observed since [`init_timer`].
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Re-arm the periodic LAPIC timer by clearing the LVT mask so ticks resume.
///
/// The heartbeat demo in [`fjord_irq_dispatch`] masks the timer after a few
/// ticks; the preemptive scheduler calls this from its tick hook to keep the
/// timer alive for as many ticks as its schedule needs. The LAPIC timer runs
/// in periodic mode, so clearing the mask is sufficient -- the initial-count
/// reload is automatic.
///
/// # Safety
/// The LAPIC window must be mapped (`init_timer` has run).
pub unsafe fn rearm_timer() {
    // SAFETY: single MMIO write to the mapped LVT timer register.
    unsafe { lapic_write(REG_LVT_TIMER, LVT_PERIODIC | TIMER_VECTOR as u32); }
}

/// Mask the periodic LAPIC timer so no further ticks are delivered.
///
/// # Safety
/// The LAPIC window must be mapped (`init_timer` has run).
pub unsafe fn mask_timer() {
    // SAFETY: single MMIO write to the mapped LVT timer register.
    unsafe {
        lapic_write(REG_LVT_TIMER, LVT_MASKED | LVT_PERIODIC | TIMER_VECTOR as u32);
    }
}

/// Rust side of the periodic timer interrupt: count, log a few, then EOI.
#[no_mangle]
extern "C" fn fjord_irq_dispatch(_frame: u64) {
    let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= TICK_DEMO_LIMIT {
        crate::kprintln!("hull: lapic timer tick {n}");
    }
    if n == TICK_DEMO_LIMIT {
        // SAFETY: masking the LVT timer is a single MMIO write to a mapped reg.
        unsafe {
            lapic_write(REG_LVT_TIMER, LVT_MASKED | LVT_PERIODIC | TIMER_VECTOR as u32);
        }
        crate::kprintln!("hull: periodic timer IRQ verified; masking to spare serial");
    }
    // SAFETY: signal end-of-interrupt so the LAPIC can deliver the next one.
    unsafe { lapic_write(REG_EOI, 0); }

    // Drive the scheduler last, after EOI: a context switch performed by the
    // hook leaves the LAPIC ready to deliver the next tick to whichever thread
    // we switch into. No-op until Keel installs a hook.
    crate::sched_hook::run_tick_hook();

    // Deliver this interrupt as a capability-backed notification to any
    // registered Keel handler. No-op until Keel installs an IRQ hook.
    crate::irq_hook::run_irq_hook(TIMER_VECTOR as u32);
}

// Hardware-interrupt entry stubs. Unlike the CPU-exception stubs in
// arch::x86_64 (which halt), these preserve all GP registers, call the Rust
// dispatcher with a 16-byte-aligned stack, restore state and `iretq`. The
// spurious stub returns immediately without an EOI (none is owed).
core::arch::global_asm!(r#"
.global fjord_timer_isr
fjord_timer_isr:
    push 0x40
    jmp fjord_irq_common

.global fjord_spurious_isr
fjord_spurious_isr:
    iretq

.global fjord_irq_common
fjord_irq_common:
    push rax
    push rbx
    push rcx
    push rdx
    push rbp
    push rdi
    push rsi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    sub rsp, 8
    lea rdi, [rsp + 8]
    call fjord_irq_dispatch
    add rsp, 8

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rsi
    pop rdi
    pop rbp
    pop rdx
    pop rcx
    pop rbx
    pop rax

    add rsp, 8
    iretq
"#);
