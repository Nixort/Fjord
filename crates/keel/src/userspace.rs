// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 28 june 2026

//! First userspace task bring-up: a one-shot unprivileged round-trip.
//!
//! This is the live user/kernel privilege boundary the Phase 2 exit criterion
//! is built on. We carve two physical frames, write a tiny user program into
//! one, map both into the *live* address space (code R-X, stack RW + NX) with
//! unprivileged access, then drop to the lowest privilege level and run until
//! the program traps with an `EXIT` syscall. The kernel recovers the magic the
//! program passed as the syscall argument, proving a complete round-trip:
//!
//! - **x86_64**: `iretq` to ring 3; the program issues `int 0x80` (DPL-3 gate).
//! - **aarch64**: `eret` to EL0; the program issues `svc #0` (lower-EL vector).
//!
//! This runs on the same resumable [`hull::user`] path that Keel's multi-task
//! syscall dispatcher ([`crate::task`]) uses — here exercised for a single
//! entry and a single trap. A scheduled pair of tasks exchanging IPC over an
//! endpoint lives in [`crate::task`].

use hull::mmu::FrameAllocator;
use hull::user::{self, UserFrame};

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
/// The `EXIT` syscall number the boundary program traps with (see [`crate::task`]).
const SYS_EXIT: u64 = 0;

/// Why the userspace round-trip could not be completed.
#[derive(Debug, Clone, Copy)]
pub enum UserError {
    /// The frame allocator could not back the code or stack page.
    OutOfFrames,
    /// A user mapping was refused (W^X violation or no table frame).
    MapFailed,
    /// The program trapped, but not with the expected `EXIT(magic)` (the value
    /// it actually passed).
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

    // Install the syscall trap path, then drop to the unprivileged level and run
    // until the program traps; recover the value it passed.
    user::init();
    let mut frame = UserFrame::new(USER_CODE_VA, USER_STACK_TOP);
    // SAFETY: the entry/stack pages were mapped user-accessible just above, and
    // `user::init` armed the trap gate / kernel stack.
    unsafe { user::run(&mut frame) };

    let echoed = frame.arg0();
    if frame.syscall_nr() != SYS_EXIT || echoed != ECHO_MAGIC {
        return Err(UserError::BadEcho(echoed));
    }
    Ok(echoed)
}

// ---------------------------------------------------------------------------
// The tiny user program: `EXIT(ECHO_MAGIC)`. Syscall number in rax/x0, the
// argument in rdi/x1 — the ABI documented in `hull::user`.
// ---------------------------------------------------------------------------

/// Write the ring-3 program: `xor eax,eax; mov edi,MAGIC; int 0x80; jmp $`.
#[cfg(target_arch = "x86_64")]
fn write_user_program(code_pa: u64) {
    //   31 C0             xor eax, eax          (rax = 0 = SYS_EXIT)
    //   BF 11 CA 0D F7    mov edi, 0xF70DCA11   (rdi = magic, zero-extended)
    //   CD 80             int 0x80
    //   EB FE             jmp $
    let blob: [u8; 9] = [0x31, 0xC0, 0xBF, 0x11, 0xCA, 0x0D, 0xF7, 0xCD, 0x80];
    // SAFETY: `code_pa` is a fresh, identity-mapped, writable RAM frame that
    // nothing else references yet; x86 caches are coherent with instruction fetch.
    unsafe {
        core::ptr::copy_nonoverlapping(blob.as_ptr(), code_pa as *mut u8, blob.len());
        // The `jmp $` lands two bytes on; write it separately to keep the blob
        // a clean instruction stream.
        core::ptr::write((code_pa + 9) as *mut u8, 0xEB);
        core::ptr::write((code_pa + 10) as *mut u8, 0xFE);
    }
}

/// Write the EL0 program: `mov x0,#0; movz w1,#0xCA11; movk w1,#0xF70D,lsl#16;
/// svc #0; b .` then publish it to the instruction side (aarch64 is not I/D
/// coherent).
#[cfg(target_arch = "aarch64")]
fn write_user_program(code_pa: u64) {
    //   D2800000   mov  x0, #0                 (x0 = SYS_EXIT)
    //   52994221   movz w1, #0xCA11
    //   72BEE1A1   movk w1, #0xF70D, lsl #16   (x1 = 0xF70DCA11)
    //   D4000001   svc  #0
    //   14000000   b    .
    let blob: [u32; 5] = [
        0xD280_0000,
        0x5299_4221,
        0x72BE_E1A1,
        0xD400_0001,
        0x1400_0000,
    ];
    // SAFETY: `code_pa` is a fresh, identity-mapped, writable RAM frame; the
    // cache maintenance publishes the bytes for the EL0 instruction fetch.
    unsafe {
        core::ptr::copy_nonoverlapping(blob.as_ptr(), code_pa as *mut u32, blob.len());
        hull::arch::aarch64::sync_instruction_cache(code_pa, core::mem::size_of_val(&blob));
    }
}
