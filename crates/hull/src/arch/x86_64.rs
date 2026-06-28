// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! x86_64 CPU bring-up: GDT, TSS and IDT.
//!
//! This module intentionally avoids external crates. It provides only the
//! minimum needed for Phase 1: load descriptor tables, install CPU exception
//! handlers, report faults over the early serial console and halt.
//!
//! Later phases should replace this with per-CPU tables, IST stacks for #DF/#NMI,
//! APIC setup, syscall/sysret entry, and user/kernel privilege transitions.

use core::arch::{asm, global_asm};
use core::mem::size_of;

const KERNEL_CODE_SELECTOR: u16 = 0x08;
const KERNEL_DATA_SELECTOR: u16 = 0x10;
const TSS_SELECTOR: u16 = 0x18;
/// Ring-3 user code segment: GDT index 5 (0x28) with RPL 3.
const USER_CODE_SELECTOR: u16 = 0x28 | 3;
/// Ring-3 user data/stack segment: GDT index 6 (0x30) with RPL 3.
const USER_DATA_SELECTOR: u16 = 0x30 | 3;

const IDT_PRESENT: u16 = 1 << 15;
const IDT_RING0: u16 = 0 << 13;
/// Gate descriptor privilege level 3: lets `int N` be issued from ring 3.
const IDT_RING3: u16 = 3 << 13;
const IDT_INTERRUPT_GATE: u16 = 0xE << 8;
const IDT_FLAGS: u16 = IDT_PRESENT | IDT_RING0 | IDT_INTERRUPT_GATE;

const TSS_STACK_SIZE: usize = 16 * 1024;

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    options: u16,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

impl IdtEntry {
    const fn missing() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            options: 0,
            offset_mid: 0,
            offset_high: 0,
            reserved: 0,
        }
    }

    fn set_handler(&mut self, handler: unsafe extern "C" fn()) {
        self.set_handler_ist(handler, 0);
    }

    /// Like [`set_handler`], but vectors through Interrupt Stack Table slot
    /// `ist_index` (1..=7), so the CPU unconditionally switches to a known-good
    /// stack before entering the gate. `ist_index == 0` keeps the interrupted
    /// stack. The index occupies bits 0..=2 of the gate options byte.
    fn set_handler_ist(&mut self, handler: unsafe extern "C" fn(), ist_index: u8) {
        let addr = handler as usize as u64;
        self.offset_low = addr as u16;
        self.selector = KERNEL_CODE_SELECTOR;
        self.options = IDT_FLAGS | (ist_index as u16 & 0x7);
        self.offset_mid = (addr >> 16) as u16;
        self.offset_high = (addr >> 32) as u32;
        self.reserved = 0;
    }

    /// Like [`set_handler`], but a DPL-3 interrupt gate so the vector can be
    /// raised by `int N` from ring 3 (used for the userspace syscall trap).
    /// The handler still runs in ring 0 via the kernel code selector.
    fn set_handler_user(&mut self, handler: unsafe extern "C" fn()) {
        let addr = handler as usize as u64;
        self.offset_low = addr as u16;
        self.selector = KERNEL_CODE_SELECTOR;
        self.options = IDT_PRESENT | IDT_RING3 | IDT_INTERRUPT_GATE;
        self.offset_mid = (addr >> 16) as u16;
        self.offset_high = (addr >> 32) as u32;
        self.reserved = 0;
    }
}

#[repr(C, packed)]
struct TaskStateSegment {
    _reserved_0: u32,
    rsp: [u64; 3],
    _reserved_1: u64,
    ist: [u64; 7],
    _reserved_2: u64,
    _reserved_3: u16,
    iopb_offset: u16,
}

impl TaskStateSegment {
    const fn new() -> Self {
        Self {
            _reserved_0: 0,
            rsp: [0; 3],
            _reserved_1: 0,
            ist: [0; 7],
            _reserved_2: 0,
            _reserved_3: 0,
            iopb_offset: size_of::<TaskStateSegment>() as u16,
        }
    }
}

#[repr(C, align(16))]
struct BootStack([u8; TSS_STACK_SIZE]);

static mut DOUBLE_FAULT_STACK: BootStack = BootStack([0; TSS_STACK_SIZE]);
static mut TSS: TaskStateSegment = TaskStateSegment::new();
static mut GDT: [u64; 7] = [
    0x0000_0000_0000_0000, // null
    0x00AF_9A00_0000_FFFF, // kernel code: base=0, limit ignored, long-mode
    0x00AF_9200_0000_FFFF, // kernel data
    0x0000_0000_0000_0000, // TSS low (filled at runtime)
    0x0000_0000_0000_0000, // TSS high
    0x00AF_FA00_0000_FFFF, // user code (DPL 3, long-mode)
    0x00AF_F200_0000_FFFF, // user data (DPL 3)
];
static mut IDT: [IdtEntry; 256] = [IdtEntry::missing(); 256];

/// CPU exception vector names (Intel SDM order, 0..31).
const EXCEPTION_NAMES: [&str; 32] = [
    "#DE divide error",
    "#DB debug",
    "NMI interrupt",
    "#BP breakpoint",
    "#OF overflow",
    "#BR bound range exceeded",
    "#UD invalid opcode",
    "#NM device not available",
    "#DF double fault",
    "coprocessor segment overrun",
    "#TS invalid TSS",
    "#NP segment not present",
    "#SS stack-segment fault",
    "#GP general protection fault",
    "#PF page fault",
    "reserved",
    "#MF x87 floating-point exception",
    "#AC alignment check",
    "#MC machine check",
    "#XM SIMD floating-point exception",
    "#VE virtualization exception",
    "#CP control protection exception",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "#HV hypervisor injection exception",
    "#VC VMM communication exception",
    "#SX security exception",
    "reserved",
];

/// Saved state passed from the common assembly exception stub.
#[repr(C)]
struct ExceptionFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    rdx: u64,
    rcx: u64,
    rbx: u64,
    rax: u64,
    vector: u64,
    error_code: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

unsafe extern "C" {
    fn fjord_isr_00();
    fn fjord_isr_01();
    fn fjord_isr_02();
    fn fjord_isr_03();
    fn fjord_isr_04();
    fn fjord_isr_05();
    fn fjord_isr_06();
    fn fjord_isr_07();
    fn fjord_isr_08();
    fn fjord_isr_09();
    fn fjord_isr_10();
    fn fjord_isr_11();
    fn fjord_isr_12();
    fn fjord_isr_13();
    fn fjord_isr_14();
    fn fjord_isr_15();
    fn fjord_isr_16();
    fn fjord_isr_17();
    fn fjord_isr_18();
    fn fjord_isr_19();
    fn fjord_isr_20();
    fn fjord_isr_21();
    fn fjord_isr_22();
    fn fjord_isr_23();
    fn fjord_isr_24();
    fn fjord_isr_25();
    fn fjord_isr_26();
    fn fjord_isr_27();
    fn fjord_isr_28();
    fn fjord_isr_29();
    fn fjord_isr_30();
    fn fjord_isr_31();
}

/// Load the boot CPU descriptor tables and install CPU exception handlers.
///
/// # Safety
/// Must be called exactly during early boot, before interrupts are enabled and
/// before multiple CPUs can concurrently mutate the static tables.
pub unsafe fn init_boot_cpu() {
    // SAFETY: early boot owns the static TSS and its bootstrap stack.
    unsafe {
        let df_stack_top =
            (&raw const DOUBLE_FAULT_STACK.0 as *const u8).add(TSS_STACK_SIZE) as u64;
        TSS.ist[0] = df_stack_top;
        write_tss_descriptor(&raw const TSS as u64);
        fill_idt();
        load_gdt();
        load_tss();
        load_idt();
    }

    crate::kprintln!("hull: x86_64 GDT/TSS/IDT loaded");
}

unsafe fn write_tss_descriptor(tss_base: u64) {
    let tss_limit = (size_of::<TaskStateSegment>() - 1) as u64;
    let low = (tss_limit & 0xFFFF)
        | ((tss_base & 0xFF_FFFF) << 16)
        | (0x89_u64 << 40)
        | (((tss_limit >> 16) & 0xF) << 48)
        | (((tss_base >> 24) & 0xFF) << 56);
    let high = tss_base >> 32;

    // SAFETY: early boot owns the mutable static GDT before SMP/interrupts.
    unsafe {
        GDT[3] = low;
        GDT[4] = high;
    }
}

unsafe fn fill_idt() {
    let handlers: [unsafe extern "C" fn(); 32] = [
        fjord_isr_00,
        fjord_isr_01,
        fjord_isr_02,
        fjord_isr_03,
        fjord_isr_04,
        fjord_isr_05,
        fjord_isr_06,
        fjord_isr_07,
        fjord_isr_08,
        fjord_isr_09,
        fjord_isr_10,
        fjord_isr_11,
        fjord_isr_12,
        fjord_isr_13,
        fjord_isr_14,
        fjord_isr_15,
        fjord_isr_16,
        fjord_isr_17,
        fjord_isr_18,
        fjord_isr_19,
        fjord_isr_20,
        fjord_isr_21,
        fjord_isr_22,
        fjord_isr_23,
        fjord_isr_24,
        fjord_isr_25,
        fjord_isr_26,
        fjord_isr_27,
        fjord_isr_28,
        fjord_isr_29,
        fjord_isr_30,
        fjord_isr_31,
    ];
    for (vector, handler) in handlers.iter().enumerate() {
        // SAFETY: early boot owns the mutable static IDT before SMP/interrupts.
        unsafe { IDT[vector].set_handler(*handler) };
    }

    // #DF (vector 8) must land on its own IST stack. A kernel-stack overflow
    // raises a fault whose handler would re-push onto the same exhausted stack
    // and immediately re-fault, escalating to a triple fault (CPU reset). IST1
    // resolves to TSS.ist[0], primed with DOUBLE_FAULT_STACK in init_boot_cpu.
    const VECTOR_DOUBLE_FAULT: usize = 8;
    const IST_DOUBLE_FAULT: u8 = 1;
    // SAFETY: early boot owns the mutable static IDT before SMP/interrupts.
    unsafe { IDT[VECTOR_DOUBLE_FAULT].set_handler_ist(fjord_isr_08, IST_DOUBLE_FAULT) };
}

unsafe fn load_gdt() {
    let pointer = DescriptorTablePointer {
        limit: (size_of::<[u64; 7]>() - 1) as u16,
        base: &raw const GDT as u64,
    };

    // SAFETY: pointer references the static GDT above. Segment reload installs
    // the new code/data descriptors for subsequent Rust code.
    unsafe {
        asm!(
            "lgdt [{gdt}]",
            "push {code}",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            "mov ax, {data}",
            "mov ds, ax",
            "mov es, ax",
            "mov ss, ax",
            "mov fs, ax",
            "mov gs, ax",
            gdt = in(reg) &pointer,
            code = const KERNEL_CODE_SELECTOR as u64,
            data = const KERNEL_DATA_SELECTOR,
            out("rax") _,
            options(preserves_flags),
        );
    }
}

unsafe fn load_tss() {
    // SAFETY: TSS descriptor is present in the loaded GDT.
    unsafe {
        asm!("ltr ax", in("ax") TSS_SELECTOR, options(nomem, nostack, preserves_flags));
    }
}

unsafe fn load_idt() {
    let pointer = DescriptorTablePointer {
        limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: &raw const IDT as u64,
    };
    // SAFETY: pointer references the static IDT above.
    unsafe {
        asm!("lidt [{idt}]", idt = in(reg) &pointer, options(readonly, nostack, preserves_flags));
    }
}

/// Install or replace an interrupt gate at `vector` after the IDT is loaded.
///
/// Used by the APIC bring-up to register hardware-IRQ entry stubs (timer,
/// spurious) on top of the CPU-exception gates installed at boot.
///
/// # Safety
/// Must run with interrupts disabled (e.g. before the first `sti`) so the IDT
/// is never observed mid-update, and `handler` must be a valid ISR entry stub
/// that follows the matching push/`iretq` convention.
pub unsafe fn set_irq_gate(vector: u8, handler: unsafe extern "C" fn()) {
    // SAFETY: single-core early boot owns the static IDT before interrupts.
    unsafe {
        IDT[vector as usize].set_handler(handler);
    }
}

/// Set the ring-0 stack pointer the CPU loads from the TSS on a privilege
/// escalation (e.g. an `int 0x80` issued from ring 3). Must point at the top of
/// a valid, exclusively-owned kernel stack.
///
/// # Safety
/// Must run during single-core early boot before the first ring-3 entry; the
/// stack must outlive every userspace transition that may use it.
pub unsafe fn set_kernel_stack(rsp0: u64) {
    // SAFETY: early boot owns the static TSS before SMP/interrupts.
    unsafe {
        TSS.rsp[0] = rsp0;
    }
}

/// Install the ring-3 `int vector` syscall trap, routing it to the resumable
/// user-excursion handler driven by [`user_run`]. Installed as a DPL-3 interrupt
/// gate so `int vector` is permitted from ring 3.
///
/// # Safety
/// Must run with interrupts disabled and the IDT loaded; pairs with [`user_run`].
pub unsafe fn install_syscall_gate(vector: u8) {
    // SAFETY: single-core early boot owns the static IDT before interrupts.
    unsafe {
        IDT[vector as usize].set_handler_user(fjord_user_syscall_isr);
    }
}

use core::mem::offset_of;

/// A resumable ring-3 register frame: everything needed to (re)enter a user
/// task exactly where it last trapped. [`user_run`] drops to ring 3 from this
/// frame; the `int 0x80` handler saves the live ring-3 state back into it.
///
/// The kernel treats it as a coroutine resume point: read the syscall arguments
/// after a trap, write a return value with [`UserFrame::set_ret`], and call
/// [`user_run`] again to resume. The syscall ABI mirrors the System V argument
/// registers: number in `rax`, args in `rdi`/`rsi`, return value in `rax`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UserFrame {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
    rflags: u64,
    rsp: u64,
}

// The assembly addresses these fields by byte offset; the checks fail the build
// before boot if the layout ever drifts.
const _: () = assert!(offset_of!(UserFrame, rax) == 0x00);
const _: () = assert!(offset_of!(UserFrame, rdi) == 0x28);
const _: () = assert!(offset_of!(UserFrame, rbp) == 0x30);
const _: () = assert!(offset_of!(UserFrame, r15) == 0x70);
const _: () = assert!(offset_of!(UserFrame, rip) == 0x78);
const _: () = assert!(offset_of!(UserFrame, rflags) == 0x80);
const _: () = assert!(offset_of!(UserFrame, rsp) == 0x88);

impl UserFrame {
    /// A fresh frame that, on the first [`user_run`], begins executing at
    /// `entry` on `stack` with `RFLAGS = IF | reserved` and all GPRs zeroed.
    #[must_use]
    pub const fn new(entry: u64, stack: u64) -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: entry,
            rflags: 0x202,
            rsp: stack,
        }
    }

    /// The syscall number the task trapped with (`rax`).
    #[must_use]
    pub const fn syscall_nr(&self) -> u64 {
        self.rax
    }

    /// The first syscall argument (`rdi`).
    #[must_use]
    pub const fn arg0(&self) -> u64 {
        self.rdi
    }

    /// The second syscall argument (`rsi`).
    #[must_use]
    pub const fn arg1(&self) -> u64 {
        self.rsi
    }

    /// Set the value the resumed task sees as its syscall return (`rax`).
    pub fn set_ret(&mut self, value: u64) {
        self.rax = value;
    }
}

unsafe extern "C" {
    fn fjord_user_run(frame: *mut UserFrame);
    fn fjord_user_syscall_isr();
}

/// Stashed kernel `rsp` for the [`user_run`] excursion: the entry trampoline
/// saves it here and the syscall handler restores it to unwind back into the
/// caller of [`user_run`].
#[no_mangle]
static mut FJORD_KERNEL_RSP: u64 = 0;

/// Pointer to the [`UserFrame`] of the task currently in ring 3; the syscall
/// handler saves the trapping register state through it.
#[no_mangle]
static mut FJORD_CURRENT_FRAME: u64 = 0;

/// Drop to ring 3 from `frame` and run until the task issues `int 0x80`, which
/// saves the live ring-3 state back into `*frame` and returns here. Works for
/// both the first entry and every resume — the frame fully describes the
/// (re)entry point.
///
/// # Safety
/// Before calling: the syscall gate must be installed ([`install_syscall_gate`]),
/// `TSS.rsp[0]` must name a valid kernel stack ([`set_kernel_stack`]), and the
/// code/stack pages the frame names must be mapped USER-accessible (code
/// executable, stack writable + non-executable) in the active address space.
/// `frame` must outlive the call.
pub unsafe fn user_run(frame: *mut UserFrame) {
    // SAFETY: the contract above is forwarded to the caller; the trampoline
    // drops to ring 3 and the int 0x80 gate unwinds back here, having written
    // the trapping state into `*frame`.
    unsafe { fjord_user_run(frame) }
}

global_asm!(
    r#"
.global fjord_user_run
fjord_user_run:
    // rdi = *mut UserFrame.
    // Save kernel callee-saved state + flags, stash rsp and the frame pointer.
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    pushfq
    mov [rip + FJORD_KERNEL_RSP], rsp
    mov [rip + FJORD_CURRENT_FRAME], rdi
    // Build the iretq frame for ring 3 (rax is a scratch; loaded for real below).
    push 0x33                 // user SS (GDT index 6 | RPL 3)
    mov rax, [rdi + 0x88]
    push rax                  // user RSP
    mov rax, [rdi + 0x80]
    push rax                  // RFLAGS
    push 0x2b                 // user CS (GDT index 5 | RPL 3)
    mov rax, [rdi + 0x78]
    push rax                  // user RIP
    // Load the user GPRs from the frame (rdi last, since it is the base).
    mov rax, [rdi + 0x00]
    mov rbx, [rdi + 0x08]
    mov rcx, [rdi + 0x10]
    mov rdx, [rdi + 0x18]
    mov rsi, [rdi + 0x20]
    mov rbp, [rdi + 0x30]
    mov r8,  [rdi + 0x38]
    mov r9,  [rdi + 0x40]
    mov r10, [rdi + 0x48]
    mov r11, [rdi + 0x50]
    mov r12, [rdi + 0x58]
    mov r13, [rdi + 0x60]
    mov r14, [rdi + 0x68]
    mov r15, [rdi + 0x70]
    mov rdi, [rdi + 0x28]
    iretq

.global fjord_user_syscall_isr
fjord_user_syscall_isr:
    // Entered from ring 3 via int 0x80 on the TSS.rsp0 stack. The CPU pushed
    // [rip, cs, rflags, user_rsp, ss]; save the full ring-3 state into the
    // current frame, then unwind back into the user_run caller.
    push rax                       // [rsp] = user rax; iret frame now at [rsp+8]
    mov rax, [rip + FJORD_CURRENT_FRAME]
    mov [rax + 0x08], rbx
    mov [rax + 0x10], rcx
    mov [rax + 0x18], rdx
    mov [rax + 0x20], rsi
    mov [rax + 0x28], rdi
    mov [rax + 0x30], rbp
    mov [rax + 0x38], r8
    mov [rax + 0x40], r9
    mov [rax + 0x48], r10
    mov [rax + 0x50], r11
    mov [rax + 0x58], r12
    mov [rax + 0x60], r13
    mov [rax + 0x68], r14
    mov [rax + 0x70], r15
    mov rcx, [rsp]                 // user rax
    mov [rax + 0x00], rcx
    mov rcx, [rsp + 0x08]          // rip
    mov [rax + 0x78], rcx
    mov rcx, [rsp + 0x18]          // rflags
    mov [rax + 0x80], rcx
    mov rcx, [rsp + 0x20]          // user rsp
    mov [rax + 0x88], rcx
    // Unwind to the kernel (mirror of fjord_user_run's prologue).
    mov rsp, [rip + FJORD_KERNEL_RSP]
    popfq
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret
"#
);

/// Rust exception entry point called by the assembly common stub.
#[no_mangle]
extern "C" fn fjord_exception_entry(frame: &ExceptionFrame) -> ! {
    let vector = frame.vector as usize;
    let name = EXCEPTION_NAMES
        .get(vector)
        .copied()
        .unwrap_or("unknown exception");

    crate::kprintln!("\n[CPU EXCEPTION] vector={vector} {name}");
    crate::kprintln!("  error = 0x{:016x}", frame.error_code);
    crate::kprintln!("  rip   = 0x{:016x}  cs = 0x{:016x}", frame.rip, frame.cs);
    crate::kprintln!("  rsp   = 0x{:016x}  ss = 0x{:016x}", frame.rsp, frame.ss);
    crate::kprintln!("  rfl   = 0x{:016x}", frame.rflags);
    crate::kprintln!("  rax   = 0x{:016x}  rbx = 0x{:016x}", frame.rax, frame.rbx);
    crate::kprintln!("  rcx   = 0x{:016x}  rdx = 0x{:016x}", frame.rcx, frame.rdx);
    halt_forever()
}

fn halt_forever() -> ! {
    loop {
        // SAFETY: `hlt` is the architectural low-power idle instruction.
        unsafe {
            asm!("cli; hlt", options(nomem, nostack, preserves_flags));
        }
    }
}

global_asm!(
    r#"
.macro ISR_NOERR vec name
.global \name
\name:
    push 0
    push \vec
    jmp fjord_isr_common
.endm

.macro ISR_ERR vec name
.global \name
\name:
    push \vec
    jmp fjord_isr_common
.endm

ISR_NOERR 0,  fjord_isr_00
ISR_NOERR 1,  fjord_isr_01
ISR_NOERR 2,  fjord_isr_02
ISR_NOERR 3,  fjord_isr_03
ISR_NOERR 4,  fjord_isr_04
ISR_NOERR 5,  fjord_isr_05
ISR_NOERR 6,  fjord_isr_06
ISR_NOERR 7,  fjord_isr_07
ISR_ERR   8,  fjord_isr_08
ISR_NOERR 9,  fjord_isr_09
ISR_ERR   10, fjord_isr_10
ISR_ERR   11, fjord_isr_11
ISR_ERR   12, fjord_isr_12
ISR_ERR   13, fjord_isr_13
ISR_ERR   14, fjord_isr_14
ISR_NOERR 15, fjord_isr_15
ISR_NOERR 16, fjord_isr_16
ISR_ERR   17, fjord_isr_17
ISR_NOERR 18, fjord_isr_18
ISR_NOERR 19, fjord_isr_19
ISR_NOERR 20, fjord_isr_20
ISR_ERR   21, fjord_isr_21
ISR_NOERR 22, fjord_isr_22
ISR_NOERR 23, fjord_isr_23
ISR_NOERR 24, fjord_isr_24
ISR_NOERR 25, fjord_isr_25
ISR_NOERR 26, fjord_isr_26
ISR_NOERR 27, fjord_isr_27
ISR_NOERR 28, fjord_isr_28
ISR_ERR   29, fjord_isr_29
ISR_ERR   30, fjord_isr_30
ISR_NOERR 31, fjord_isr_31

.global fjord_isr_common
fjord_isr_common:
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

    mov rdi, rsp
    call fjord_exception_entry

    ud2
"#
);
