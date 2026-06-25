// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 24 june 2026

//! aarch64 interrupt + timer bring-up: the GIC v2 distributor / CPU interface
//! plus the ARM generic timer (EL1 physical timer, PPI INTID 30).
//!
//! This is the aarch64 counterpart of the x86_64 [`crate::apic`] module: arm a
//! periodic tick, log the first few interrupts to prove end-to-end delivery,
//! then mask the timer to spare the serial console. The MMU is still off during
//! this slice, so GIC MMIO is issued directly to physical addresses (treated as
//! Device memory under the MMU-off regime, hence the `strict-align` codegen).

use core::sync::atomic::{AtomicU64, Ordering};

/// GIC v2 distributor base on the QEMU `virt` machine.
const GICD_BASE: usize = 0x0800_0000;
/// GIC v2 CPU-interface base on the QEMU `virt` machine.
const GICC_BASE: usize = 0x0801_0000;

// Distributor register offsets (from GICD_BASE).
const GICD_CTLR: usize = 0x000;
const GICD_ISENABLER: usize = 0x100;
const GICD_IPRIORITYR: usize = 0x400;

// CPU-interface register offsets (from GICC_BASE).
const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_IAR: usize = 0x00C;
const GICC_EOIR: usize = 0x010;

/// EL1 physical-timer PPI (private peripheral interrupt) ID on the GIC.
const TIMER_INTID: u32 = 30;
/// The IAR INTID field is 10 bits wide.
const INTID_MASK: u32 = 0x3FF;
/// INTIDs >= 1020 are spurious / special and have no active interrupt to EOI.
const SPURIOUS_INTID: u32 = 1020;

/// Tick rate as a fraction of the counter frequency (100 Hz => 10 ms period).
const TICK_HZ: u64 = 100;
/// Log this many ticks, then mask the timer to keep the console readable.
const TICK_DEMO_LIMIT: u64 = 5;

static TICKS: AtomicU64 = AtomicU64::new(0);
/// Down-counter reload value (`CNTFRQ_EL0 / TICK_HZ`), cached for re-arming.
static INTERVAL: AtomicU64 = AtomicU64::new(0);

#[inline]
unsafe fn mmio_write(base: usize, off: usize, val: u32) {
    // SAFETY: the caller passes a valid GIC register offset; the MMU-off view
    // makes the distributor / CPU-interface MMIO directly addressable.
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, val) }
}

#[inline]
unsafe fn mmio_read(base: usize, off: usize) -> u32 {
    // SAFETY: the caller passes a valid GIC register offset; the MMU-off view
    // makes the distributor / CPU-interface MMIO directly addressable.
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}

#[inline]
fn cntfrq() -> u64 {
    let v: u64;
    // SAFETY: CNTFRQ_EL0 is readable at EL1 and has no side effects.
    unsafe {
        core::arch::asm!("mrs {}, cntfrq_el0", out(reg) v, options(nomem, nostack, preserves_flags));
    }
    v
}

#[inline]
unsafe fn set_timer_tval(ticks: u64) {
    // SAFETY: writing CNTP_TVAL_EL0 re-arms the EL1 physical timer
    // (CompareValue = PhysicalCount + ticks) and deasserts a pending tick.
    unsafe {
        core::arch::asm!("msr cntp_tval_el0, {}", in(reg) ticks, options(nomem, nostack, preserves_flags));
    }
}

#[inline]
unsafe fn set_timer_ctl(val: u64) {
    // SAFETY: CNTP_CTL_EL0 controls ENABLE (bit 0) and IMASK (bit 1) of the
    // EL1 physical timer.
    unsafe {
        core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) val, options(nomem, nostack, preserves_flags));
    }
}

/// Number of timer interrupts observed since [`init_timer`].
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Re-arm the generic timer for another tick, overriding the heartbeat demo's
/// mask so the preemptive scheduler keeps receiving ticks.
///
/// # Safety
/// The GIC and generic timer must have been initialised by [`init_timer`].
pub unsafe fn rearm_timer() {
    // SAFETY: reload the down-counter and assert ENABLE (clearing IMASK).
    unsafe {
        set_timer_tval(INTERVAL.load(Ordering::Relaxed));
        set_timer_ctl(1);
    }
}

/// Disable the generic timer so no further ticks are delivered.
///
/// # Safety
/// The generic timer must have been initialised by [`init_timer`].
pub unsafe fn disable_timer() {
    // SAFETY: clearing ENABLE stops the EL1 physical timer from firing.
    unsafe { set_timer_ctl(0); }
}

/// Bring up the GIC v2 and arm the generic-timer periodic tick.
///
/// Returns `false` if the platform reports a zero counter frequency (no usable
/// timer). Otherwise it enables the distributor and CPU interface, routes the
/// EL1 physical-timer PPI, programs the first interval, and unmasks IRQs at the
/// PSTATE level so the tick can be delivered.
pub fn init_timer() -> bool {
    let freq = cntfrq();
    if freq == 0 {
        return false;
    }
    let interval = freq / TICK_HZ;
    INTERVAL.store(interval, Ordering::Relaxed);

    // SAFETY: single-core early boot owns the GIC; MMIO is directly addressable
    // while the MMU is off, and the system registers are accessible at EL1.
    unsafe {
        // Distributor: enable forwarding, give the timer PPI a priority below
        // the CPU-interface mask, then set-enable the interrupt.
        mmio_write(GICD_BASE, GICD_CTLR, 1);
        let prio_reg = GICD_IPRIORITYR + (TIMER_INTID as usize & !0x3);
        let shift = (TIMER_INTID as usize & 0x3) * 8;
        let mut prio = mmio_read(GICD_BASE, prio_reg);
        prio &= !(0xFF_u32 << shift);
        prio |= 0x80_u32 << shift;
        mmio_write(GICD_BASE, prio_reg, prio);
        mmio_write(GICD_BASE, GICD_ISENABLER, 1_u32 << TIMER_INTID);

        // CPU interface: admit every priority below 0xF0, then enable.
        mmio_write(GICC_BASE, GICC_PMR, 0xF0);
        mmio_write(GICC_BASE, GICC_CTLR, 1);

        // Arm the EL1 physical timer (ENABLE=1, IMASK=0) and unmask IRQs.
        set_timer_tval(interval);
        set_timer_ctl(1);
        core::arch::asm!("msr daifclr, #2", options(nomem, nostack, preserves_flags));
    }
    true
}

/// Rust side of the EL1 IRQ vector: acknowledge at the GIC, service the timer,
/// then signal end-of-interrupt. Invoked by the `el1_irq` assembly trampoline.
#[no_mangle]
extern "C" fn fjord_aarch64_irq() {
    // SAFETY: reading IAR acknowledges the highest-priority pending interrupt.
    let iar = unsafe { mmio_read(GICC_BASE, GICC_IAR) };
    let intid = iar & INTID_MASK;

    if intid >= SPURIOUS_INTID {
        return; // spurious: no active interrupt, nothing to EOI
    }

    if intid == TIMER_INTID {
        let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= TICK_DEMO_LIMIT {
            crate::kprintln!("hull: generic timer tick {n}");
        }
        // SAFETY: re-arm the timer for the next tick, or disable it once the
        // demo limit is reached. Either path deasserts the current interrupt
        // before the EOI below, so it does not immediately re-fire.
        unsafe {
            if n >= TICK_DEMO_LIMIT {
                set_timer_ctl(0);
                if n == TICK_DEMO_LIMIT {
                    crate::kprintln!(
                        "hull: periodic timer IRQ verified; masking to spare serial"
                    );
                }
            } else {
                set_timer_tval(INTERVAL.load(Ordering::Relaxed));
            }
        }
    }

    // SAFETY: EOIR must echo the IAR value to drop the running priority so the
    // CPU interface can deliver the next interrupt.
    unsafe { mmio_write(GICC_BASE, GICC_EOIR, iar) };

    // Drive the scheduler last, after EOI: a context switch performed by the
    // hook resumes another thread with the GIC ready to deliver its next tick.
    // No-op until Keel installs a hook.
    crate::sched_hook::run_tick_hook();
}

// EL1 IRQ trampoline, branched to from the boot vector table's "Current EL SPx
// IRQ" slot. It saves the caller-saved general registers (the Rust dispatcher
// preserves x19..x28 per AAPCS), calls the dispatcher, restores state and
// returns from the exception. The 192-byte frame keeps SP 16-byte aligned.
//
// ELR_EL1 and SPSR_EL1 are system registers, not stacked by the CPU on an
// aarch64 exception. They MUST be saved into this per-thread frame and restored
// before `eret`: the scheduler hook can switch contexts inside the dispatcher,
// so a resumed thread has to `eret` with *its own* return PC and PSTATE, not
// those of whichever thread was interrupted most recently. Without this,
// preemptive switching erets to a stale PC and faults. (x86_64 is immune
// because its iretq frame lives on the stack.)
core::arch::global_asm!(r#"
.section .text.vectors, "ax"
.global el1_irq
el1_irq:
    sub     sp, sp, #192
    stp     x0,  x1,  [sp, #16*0]
    stp     x2,  x3,  [sp, #16*1]
    stp     x4,  x5,  [sp, #16*2]
    stp     x6,  x7,  [sp, #16*3]
    stp     x8,  x9,  [sp, #16*4]
    stp     x10, x11, [sp, #16*5]
    stp     x12, x13, [sp, #16*6]
    stp     x14, x15, [sp, #16*7]
    stp     x16, x17, [sp, #16*8]
    stp     x18, x29, [sp, #16*9]
    mrs     x0, elr_el1
    mrs     x1, spsr_el1
    stp     x0,  x1,  [sp, #16*10]
    str     x30, [sp, #16*11]

    bl      fjord_aarch64_irq

    ldp     x0,  x1,  [sp, #16*10]
    msr     elr_el1, x0
    msr     spsr_el1, x1
    ldp     x0,  x1,  [sp, #16*0]
    ldp     x2,  x3,  [sp, #16*1]
    ldp     x4,  x5,  [sp, #16*2]
    ldp     x6,  x7,  [sp, #16*3]
    ldp     x8,  x9,  [sp, #16*4]
    ldp     x10, x11, [sp, #16*5]
    ldp     x12, x13, [sp, #16*6]
    ldp     x14, x15, [sp, #16*7]
    ldp     x16, x17, [sp, #16*8]
    ldp     x18, x29, [sp, #16*9]
    ldr     x30, [sp, #16*11]
    add     sp, sp, #192
    eret
"#);
