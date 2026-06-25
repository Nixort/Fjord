// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 25 june 2026

//! First userspace task bring-up (x86_64): a one-shot ring-3 round-trip.
//!
//! This is the live user/kernel privilege boundary the Phase 2 exit criterion
//! is built on. We carve two physical frames, write a tiny ring-3 program into
//! one, map both USER-accessible into the *live* address space (code R-X+U,
//! stack RW+U+NX), then drop to ring 3 via [`hull::arch::x86_64::enter_user`].
//! The program loads a magic value and traps with `int 0x80`; the kernel
//! recovers that value and unwinds back here, proving a complete ring 3 -> ring
//! 0 transition.
//!
//! What this is *not* yet: a scheduled task with its own CSpace/TCB exchanging
//! IPC over an endpoint. That is the remainder of the Phase 2 exit criterion
//! and builds on this boundary (see `docs/ROADMAP.md`).

use hull::mmu::FrameAllocator;

/// Virtual address the ring-3 code page is mapped at. Lives in a PML4 slot the
/// kernel identity map never touches (it only uses the low 1 GiB / PML4[0]), so
/// relaxing this subtree to USER cannot widen access to any kernel page.
const USER_CODE_VA: u64 = 0x80_0000_0000;
/// Virtual address of the ring-3 stack page (same fresh subtree as the code).
const USER_STACK_VA: u64 = 0x80_0000_2000;
/// Top of the ring-3 stack (exclusive); grows down from here.
const USER_STACK_TOP: u64 = USER_STACK_VA + 0x1000;
/// Software-interrupt vector the ring-3 program traps through.
const SYSCALL_VECTOR: u8 = 0x80;
/// Sentinel the ring-3 program passes in `eax`; echoed back to prove the trip.
const ECHO_MAGIC: u64 = 0xF70D_CA11;

/// Why the userspace round-trip could not be completed.
#[derive(Debug, Clone, Copy)]
pub enum UserError {
    /// The frame allocator could not back the code or stack page.
    OutOfFrames,
    /// A USER mapping was refused (W^X violation or no table frame).
    MapFailed,
    /// Ring 3 returned, but with an unexpected value (got).
    BadEcho(u64),
}

/// Ring-0 stack the CPU switches to on the `int 0x80` privilege escalation.
#[repr(C, align(16))]
struct SyscallStack([u8; 16 * 1024]);
static mut SYSCALL_STACK: SyscallStack = SyscallStack([0; 16 * 1024]);

/// Run the first-userspace-task round-trip and return the echoed magic.
///
/// Consumes two frames from `frames` (permanent; the bump allocator never
/// recycles). Safe to call once during early boot after the address space and
/// IDT are live.
pub fn selftest(frames: &mut FrameAllocator) -> Result<u64, UserError> {
    let code_pa = frames.alloc().ok_or(UserError::OutOfFrames)?;
    let stack_pa = frames.alloc().ok_or(UserError::OutOfFrames)?;

    // Tiny ring-3 program. It never returns to itself — the kernel unwinds the
    // round-trip on the trap — but the trailing `jmp $` keeps it well-formed.
    //   B8 11 CA 0D F7   mov eax, 0xF70DCA11   (imm32, little-endian)
    //   CD 80            int 0x80
    //   EB FE            jmp $
    let blob: [u8; 9] = [0xB8, 0x11, 0xCA, 0x0D, 0xF7, 0xCD, 0x80, 0xEB, 0xFE];
    // SAFETY: `code_pa` is a fresh, identity-mapped, writable RAM frame that
    // nothing else references yet.
    unsafe {
        core::ptr::copy_nonoverlapping(blob.as_ptr(), code_pa as *mut u8, blob.len());
    }

    // Map the program and its stack USER-accessible into the live address space.
    let root = hull::paging::active_root();
    // SAFETY: `root` is the live kernel PML4 (from CR3); its tables are all
    // identity-mapped and reachable by physical address.
    let mut mapper = unsafe { hull::paging::Mapper::from_root(root) };
    // Code page: executable, read-only, USER.
    if !hull::paging::map_user_page(&mut mapper, USER_CODE_VA, code_pa, false, true, frames) {
        return Err(UserError::MapFailed);
    }
    // Stack page: writable, non-executable, USER.
    if !hull::paging::map_user_page(&mut mapper, USER_STACK_VA, stack_pa, true, false, frames) {
        return Err(UserError::MapFailed);
    }
    // SAFETY: drop any stale translation for the freshly mapped user pages.
    unsafe {
        hull::paging::flush_tlb_page(USER_CODE_VA);
        hull::paging::flush_tlb_page(USER_STACK_VA);
    }

    // Point the TSS at a kernel stack for the ring 3 -> ring 0 trap transition,
    // then open the DPL-3 syscall gate.
    // SAFETY: single-core early boot; SYSCALL_STACK outlives the transition and
    // the gate pairs with enter_user below.
    let kstack_top = unsafe {
        let base = &raw const SYSCALL_STACK as *const _ as *const u8;
        base.add(core::mem::size_of::<SyscallStack>()) as u64
    };
    // SAFETY: see above; both calls touch only the static TSS/IDT before SMP.
    unsafe {
        hull::arch::x86_64::set_kernel_stack(kstack_top);
        hull::arch::x86_64::install_syscall_gate(SYSCALL_VECTOR);
    }

    // Drop to ring 3 and run until the program traps; recover the echoed value.
    // SAFETY: gate installed, kernel stack set, and the entry/stack pages were
    // mapped USER-accessible just above.
    let echoed = unsafe { hull::arch::x86_64::enter_user(USER_CODE_VA, USER_STACK_TOP) };
    if echoed != ECHO_MAGIC {
        return Err(UserError::BadEcho(echoed));
    }
    Ok(echoed)
}
