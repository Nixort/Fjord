// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 25 june 2026

//! IRQ delivery as capabilities: the seL4 IRQHandler model.
//!
//! In a capability microkernel the kernel does not *service* interrupts -- it
//! *delivers* them. Each hardware interrupt line is bound to a
//! [`Notification`] (§3.1 of the architecture doc) with a badge bit. When the
//! interrupt fires, the kernel signals the notification (OR-ing the badge
//! into the pending word) and returns; the driver thread, which is blocked on
//! `poll()`, wakes, services the hardware, and re-arms.
//!
//! This module is a heap-free, static-table model of that mechanism. It is the
//! last Phase 2 piece: proving that an interrupt observed by Hull can be
//! delivered to a Keel-side [`Notification`] purely through the
//! inversion-of-control seam in [`hull::irq_hook`], without Hull depending on
//! Keel.
//!
//! See `docs/ARCHITECTURE.md` §1 ("Обработка прерываний → доставка как
//! IPC/notification").

use crate::ipc::Notification;
use core::sync::atomic::{AtomicU64, Ordering};

/// Counter incremented each time [`dispatch`] delivers a notification.
/// Used by the self-test to avoid depending on the heartbeat tick counter,
/// which may have been masked by the demo limit on aarch64.
static DELIVERED: AtomicU64 = AtomicU64::new(0);

/// Maximum number of interrupt lines the static table can hold.
///
/// Small but sufficient for the boot self-test and early drivers. Grown by
/// re-typing from untyped memory in a later phase, as with all Keel objects.
pub const MAX_IRQ_LINES: usize = 16;

/// One entry in the IRQ handler table: a binding from `irq_num` to a
/// [`Notification`] and the badge bit to signal on delivery.
///
/// `notification` is a raw pointer because the table is statically allocated
/// and the notification lives in caller-owned storage (ultimately a frame
/// re-typed from untyped memory). The pointer is never dereferenced after the
/// notification is freed -- `unregister` clears `active` first.
#[derive(Clone, Copy)]
struct IrqBinding {
    /// The architecture-specific interrupt number (LAPIC vector / GIC INTID).
    irq_num: u32,
    /// The notification to signal when this IRQ fires.
    notification: *mut Notification,
    /// The badge bit to OR into the notification's pending word.
    badge: u64,
    /// Whether this binding is live (registered and not yet unregistered).
    active: bool,
}

impl IrqBinding {
    /// A const-constructible empty binding for static array initialisation.
    const DEFAULT: Self = Self {
        irq_num: 0,
        notification: core::ptr::null_mut(),
        badge: 0,
        active: false,
    };
}

impl Default for IrqBinding {
    fn default() -> Self {
        Self::DEFAULT
    }
}

// SAFETY: `IrqBinding` holds a raw pointer to a `Notification` that is
// `Send`-safe because it is only accessed from the single boot CPU (no
// multi-core yet) and registration/teardown happens with interrupts quiesced.
unsafe impl Send for IrqBinding {}

/// The static IRQ handler table. Heap-free; lives in `.bss`.
static mut TABLE: [IrqBinding; MAX_IRQ_LINES] = [IrqBinding::DEFAULT; MAX_IRQ_LINES];

/// Why an IRQ-handler operation was refused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IrqError {
    /// The static table is full (all [`MAX_IRQ_LINES`] slots are active).
    TableFull,
    /// No binding for `irq_num` was found.
    NotFound,
    /// A binding for `irq_num` already exists (interrupt is already registered).
    AlreadyRegistered,
}

/// Register a binding: when `irq_num` fires, `notification.signal(badge)` is
/// called from the ISR via the Hull IRQ hook.
///
/// # Errors
/// - [`IrqError::TableFull`] if the table is full.
/// - [`IrqError::AlreadyRegistered`] if `irq_num` is already bound.
///
/// # Safety
/// `notification` must point to a valid `Notification` that outlives the
/// binding. The caller must ensure no concurrent access to the table (the
/// boot self-test and early bring-up are single-CPU with interrupts quiesced).
pub fn register(irq_num: u32, notification: &mut Notification, badge: u64) -> Result<(), IrqError> {
    // SAFETY: single CPU, interrupts quiesced during registration.
    let table = unsafe { &mut *(&raw mut TABLE) };

    // Reject duplicate registration of the same IRQ line.
    for slot in table.iter() {
        if slot.active && slot.irq_num == irq_num {
            return Err(IrqError::AlreadyRegistered);
        }
    }

    // Find the first inactive slot.
    for slot in table.iter_mut() {
        if !slot.active {
            *slot = IrqBinding {
                irq_num,
                notification: notification as *mut Notification,
                badge,
                active: true,
            };
            return Ok(());
        }
    }

    Err(IrqError::TableFull)
}

/// Remove the binding for `irq_num` so it no longer delivers notifications.
///
/// # Errors
/// [`IrqError::NotFound`] if `irq_num` is not registered.
pub fn unregister(irq_num: u32) -> Result<(), IrqError> {
    // SAFETY: single CPU, interrupts quiesced during teardown.
    let table = unsafe { &mut *(&raw mut TABLE) };
    for slot in table.iter_mut() {
        if slot.active && slot.irq_num == irq_num {
            *slot = IrqBinding::default();
            return Ok(());
        }
    }
    Err(IrqError::NotFound)
}

/// Deliver `irq_num` to its registered notification. Called from the Hull IRQ
/// hook (invoked by the timer ISR after EOI).
///
/// If no binding exists for `irq_num`, this is a no-op -- the interrupt is
/// simply dropped, which is safe (the EOI has already been issued).
///
/// # Safety
/// This is called from interrupt context. The notification `signal` is
/// non-blocking (a single atomic-ish OR of a word), so it is safe to call
/// from the ISR. The caller must ensure the `Notification` pointers in the
/// table are still valid (not freed).
pub fn dispatch(irq_num: u32) {
    // SAFETY: called from ISR; the table is only mutated during registration
    // which happens with interrupts quiesced, so the ISR-side read is safe.
    let table = unsafe { &*(&raw const TABLE) };
    for slot in table.iter() {
        if slot.active && slot.irq_num == irq_num {
            // SAFETY: `notification` was set by `register` to a valid pointer
            // that outlives the binding. `signal` is a non-blocking word OR.
            unsafe {
                (*slot.notification).signal(slot.badge);
            }
            DELIVERED.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
    // No binding for this IRQ: silently drop. EOI has already been issued.
}

/// The `extern "C"` shim that Hull calls from the timer ISR. It forwards to
/// [`dispatch`]. Keel registers this with [`hull::irq_hook::set_irq_hook`].
///
/// This is the IRQ-delivery counterpart of `tide::preempt::on_tick`: it is the
/// function whose address is stuffed into the Hull hook.
extern "C" fn irq_dispatch_shim(irq: u32) {
    dispatch(irq);
}

/// Boot-time self-test for the IRQ-as-capability delivery mechanism.
///
/// Registers a mock notification on the platform timer interrupt, arms the
/// timer for a few ticks, and verifies that the notification received the
/// expected badge bits. Uses the Hull IRQ hook seam (inversion of control).
///
/// # Errors
/// Returns [`IrqError`] if any invariant fails.
///
/// # Safety
/// This function arms and disarms the platform timer. It must be called after
/// the interrupt controller (APIC/GIC) is initialised and before the idle loop.
pub fn selftest() -> Result<u64, IrqError> {
    // The platform timer IRQ number: LAPIC vector 0x40 on x86_64, GIC INTID 30
    // on aarch64.
    #[cfg(target_arch = "x86_64")]
    const TIMER_IRQ: u32 = 0x40;
    #[cfg(target_arch = "aarch64")]
    const TIMER_IRQ: u32 = 30;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    const TIMER_IRQ: u32 = 0;

    /// Badge bit for the self-test notification.
    const TEST_BADGE: u64 = 1 << 8; // bit 8, above the heartbeat counters

    // Fresh notification on the stack.
    let mut notif = Notification::new();

    // Register the binding.
    register(TIMER_IRQ, &mut notif, TEST_BADGE)?;

    // Install the Hull IRQ hook so timer interrupts reach our dispatch.
    hull::irq_hook::set_irq_hook(irq_dispatch_shim);

    // Arm the timer and let it fire a few times. We use the existing
    // heartbeat mechanism: re-arm, spin briefly, then check.
    //
    // SAFETY: the APIC/GIC timer has been initialised by `activate_address_space`.
    // We re-arm it here and let it fire. After the test we disable it.
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: APIC is mapped and initialised.
        unsafe { hull::apic::rearm_timer() };
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: GIC and generic timer are initialised.
        unsafe { hull::gic::rearm_timer() };
    }

    // Spin until dispatch() increments DELIVERED (at least 1 delivery),
    // or we time out. We cannot rely on a raw tick counter because the
    // heartbeat demo may have masked the timer on aarch64 (n >= 5 =>
    // set_timer_ctl(0) on every subsequent tick), which would prevent
    // the counter from advancing. The DELIVERED counter is incremented
    // inside dispatch() itself, so it counts actual badge deliveries,
    // not raw timer ticks.
    let deadline = 5_000_000u64;
    let mut spun = 0u64;
    loop {
        if DELIVERED.load(Ordering::Relaxed) >= 1 {
            break;
        }
        spun += 1;
        if spun >= deadline {
            // Timer did not fire / dispatch did not run; abort.
            hull::irq_hook::clear_irq_hook();
            let _ = unregister(TIMER_IRQ);
            return Err(IrqError::NotFound);
        }
        core::hint::spin_loop();
    }

    // Disable the timer so no more ticks interfere.
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: APIC is mapped.
        unsafe { hull::apic::mask_timer() };
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: generic timer is initialised.
        unsafe { hull::gic::disable_timer() };
    }

    // Tear down the hook.
    hull::irq_hook::clear_irq_hook();

    // Check the notification received the badge.
    let received = notif.poll();
    let _ = unregister(TIMER_IRQ);

    if received & TEST_BADGE != 0 {
        Ok(received)
    } else {
        Err(IrqError::NotFound)
    }
}
