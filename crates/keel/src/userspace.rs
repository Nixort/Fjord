// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 25 june 2026

//! First userspace task bring-up: a one-shot unprivileged round-trip.
//!
//! This is the live user/kernel privilege boundary the Phase 2 exit criterion
//! is built on. We carve two physical frames, write a tiny user program into
//! one, map both into the *live* address space (code R-X, stack RW + NX) with
//! unprivileged access, then drop to the lowest privilege level and run until
//! the program traps. The kernel recovers the magic value the program passed
//! and unwinds back here, proving a complete round-trip across the boundary:
//!
//! - **x86_64**: `iretq` to ring 3; the program issues `int 0x80` (DPL-3 gate).
//! - **aarch64**: `eret` to EL0; the program issues `svc #0` (lower-EL vector).
//!
//! What this is *not* yet: a scheduled task with its own CSpace/TCB exchanging
//! IPC over an endpoint. That is the remainder of the Phase 2 exit criterion
//! and builds on this boundary (see `docs/ROADMAP.md`).

use hull::mmu::FrameAllocator;

/// Virtual address the user code page is mapped at. Lives in a translation
/// subtree the kernel identity map never touches, so making it unprivileged
/// cannot widen access to any kernel page.
const USER_CODE_VA: u64 = 0x80_0000_0000;
/// Virtual address of the user stack page (same fresh subtree as the code).
const USER_STACK_VA: u64 = 0x80_0000_2000;
/// Top of the user stack (exclusive); grows down from here.
const USER_STACK_TOP: u64 = USER_STACK_VA + 0x1000;
/// Sentinel the user program passes back; echoed to prove the trip.
const ECHO_MAGIC: u64 = 0xF70D_CA11;

/// Why the userspace round-trip could not be completed.
#[derive(Debug, Clone, Copy)]
pub enum UserError {
    /// The frame allocator could not back the code or stack page.
    OutOfFrames,
    /// A user mapping was refused (W^X violation or no table frame).
    MapFailed,
    /// The program returned, but with an unexpected value (the value it gave).
    BadEcho(u64),
}

/// Run the first-userspace-task round-trip and return the echoed magic.
///
/// Consumes two frames from `frames` (permanent; the bump allocator never
/// recycles). Safe to call once during early boot after the address space and
/// CPU vectors are live.
pub fn selftest(frames: &mut FrameAllocator) -> Result<u64, UserError> {
    let code_pa = frames.alloc().ok_or(UserError::OutOfFrames)?;
    let stack_pa = frames.alloc().ok_or(UserError::OutOfFrames)?;

    write_user_program(code_pa);

    // Map the program (executable, read-only) and its stack (writable, NX)
    // unprivileged into the live address space.
    let root = hull::paging::active_root();
    // SAFETY: `root` is the live kernel root (CR3 / TTBR0_EL1); its tables are
    // all reachable at their identity-mapped physical addresses.
    let mut mapper = unsafe { hull::paging::Mapper::from_root(root) };
    if !hull::paging::map_user_page(&mut mapper, USER_CODE_VA, code_pa, false, true, frames) {
        return Err(UserError::MapFailed);
    }
    if !hull::paging::map_user_page(&mut mapper, USER_STACK_VA, stack_pa, true, false, frames) {
        return Err(UserError::MapFailed);
    }
    // SAFETY: only drops any stale translation for the freshly mapped pages.
    unsafe {
        hull::paging::flush_tlb_page(USER_CODE_VA);
        hull::paging::flush_tlb_page(USER_STACK_VA);
    }

    // Drop to the unprivileged level and run until the program traps; recover
    // the echoed value.
    // SAFETY: the entry/stack pages were mapped unprivileged just above, and
    // `enter` installs any per-arch trap state before the transition.
    let echoed = unsafe { enter(USER_CODE_VA, USER_STACK_TOP) };
    if echoed != ECHO_MAGIC {
        return Err(UserError::BadEcho(echoed));
    }
    Ok(echoed)
}

// ---------------------------------------------------------------------------
// x86_64: ring 3 via `iretq`, trap back through a DPL-3 `int 0x80` gate.
// ---------------------------------------------------------------------------

/// Software-interrupt vector the ring-3 program traps through.
#[cfg(target_arch = "x86_64")]
const SYSCALL_VECTOR: u8 = 0x80;

/// Ring-0 stack the CPU switches to on the `int 0x80` privilege escalation.
#[cfg(target_arch = "x86_64")]
#[repr(C, align(16))]
struct SyscallStack([u8; 16 * 1024]);
#[cfg(target_arch = "x86_64")]
static mut SYSCALL_STACK: SyscallStack = SyscallStack([0; 16 * 1024]);

/// Write the ring-3 program: `mov eax, MAGIC; int 0x80; jmp $`.
#[cfg(target_arch = "x86_64")]
fn write_user_program(code_pa: u64) {
    //   B8 11 CA 0D F7   mov eax, 0xF70DCA11   (imm32, little-endian)
    //   CD 80            int 0x80
    //   EB FE            jmp $
    let blob: [u8; 9] = [0xB8, 0x11, 0xCA, 0x0D, 0xF7, 0xCD, 0x80, 0xEB, 0xFE];
    // SAFETY: `code_pa` is a fresh, identity-mapped, writable RAM frame that
    // nothing else references yet; x86 caches are coherent with instruction fetch.
    unsafe {
        core::ptr::copy_nonoverlapping(blob.as_ptr(), code_pa as *mut u8, blob.len());
    }
}

/// Prepare the trap path (kernel stack + DPL-3 gate), then drop to ring 3 and
/// return the value the program traps with.
///
/// # Safety
/// The entry/stack pages must be mapped USER-accessible; single-shot early-boot
/// use before SMP.
#[cfg(target_arch = "x86_64")]
unsafe fn enter(entry: u64, stack: u64) -> u64 {
    // SAFETY: single-core early boot; SYSCALL_STACK outlives the transition and
    // the gate pairs with the `int 0x80` in the user program.
    unsafe {
        let base = &raw const SYSCALL_STACK as *const u8;
        let kstack_top = base.add(core::mem::size_of::<SyscallStack>()) as u64;
        hull::arch::x86_64::set_kernel_stack(kstack_top);
        hull::arch::x86_64::install_syscall_gate(SYSCALL_VECTOR);
        hull::arch::x86_64::enter_user(entry, stack)
    }
}

// ---------------------------------------------------------------------------
// aarch64: EL0 via `eret`, trap back through the lower-EL `svc #0` vector.
// ---------------------------------------------------------------------------

/// Write the EL0 program: `movz w0,#0xCA11; movk w0,#0xF70D,lsl#16; svc #0; b .`
/// then publish it to the instruction side (aarch64 is not I/D coherent).
#[cfg(target_arch = "aarch64")]
fn write_user_program(code_pa: u64) {
    //   52994220   movz w0, #0xCA11
    //   72BEE1A0   movk w0, #0xF70D, lsl #16   (w0 = 0xF70DCA11)
    //   D4000001   svc  #0
    //   14000000   b    .                      (well-formed if ever resumed)
    let blob: [u32; 4] = [0x5299_4220, 0x72BE_E1A0, 0xD400_0001, 0x1400_0000];
    // SAFETY: `code_pa` is a fresh, identity-mapped, writable RAM frame; the
    // cache maintenance publishes the bytes for the EL0 instruction fetch.
    unsafe {
        core::ptr::copy_nonoverlapping(blob.as_ptr(), code_pa as *mut u32, blob.len());
        hull::arch::aarch64::sync_instruction_cache(code_pa, core::mem::size_of_val(&blob));
    }
}

/// Drop to EL0 and return the value the program traps with via `svc #0`.
///
/// # Safety
/// The entry/stack pages must be mapped EL0-accessible; the boot vector table
/// routes the EL0 `svc` to `el0_sync`. Single-shot early-boot use.
#[cfg(target_arch = "aarch64")]
unsafe fn enter(entry: u64, stack: u64) -> u64 {
    // SAFETY: see contract; the trampoline saves/restores callee-saved state.
    unsafe { hull::arch::aarch64::enter_user(entry, stack) }
}
