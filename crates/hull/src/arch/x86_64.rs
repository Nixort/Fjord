// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
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

const IDT_PRESENT: u16 = 1 << 15;
const IDT_RING0: u16 = 0 << 13;
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
static mut GDT: [u64; 5] = [
    0x0000_0000_0000_0000, // null
    0x00AF_9A00_0000_FFFF, // kernel code: base=0, limit ignored, long-mode
    0x00AF_9200_0000_FFFF, // kernel data
    0x0000_0000_0000_0000, // TSS low (filled at runtime)
    0x0000_0000_0000_0000, // TSS high
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
        let df_stack_top = (&raw const DOUBLE_FAULT_STACK.0 as *const u8)
            .add(TSS_STACK_SIZE) as u64;
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
        fjord_isr_00, fjord_isr_01, fjord_isr_02, fjord_isr_03,
        fjord_isr_04, fjord_isr_05, fjord_isr_06, fjord_isr_07,
        fjord_isr_08, fjord_isr_09, fjord_isr_10, fjord_isr_11,
        fjord_isr_12, fjord_isr_13, fjord_isr_14, fjord_isr_15,
        fjord_isr_16, fjord_isr_17, fjord_isr_18, fjord_isr_19,
        fjord_isr_20, fjord_isr_21, fjord_isr_22, fjord_isr_23,
        fjord_isr_24, fjord_isr_25, fjord_isr_26, fjord_isr_27,
        fjord_isr_28, fjord_isr_29, fjord_isr_30, fjord_isr_31,
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
        limit: (size_of::<[u64; 5]>() - 1) as u16,
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
    unsafe { asm!("ltr ax", in("ax") TSS_SELECTOR, options(nomem, nostack, preserves_flags)); }
}

unsafe fn load_idt() {
    let pointer = DescriptorTablePointer {
        limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: &raw const IDT as u64,
    };
    // SAFETY: pointer references the static IDT above.
    unsafe { asm!("lidt [{idt}]", idt = in(reg) &pointer, options(readonly, nostack, preserves_flags)); }
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
    unsafe { IDT[vector as usize].set_handler(handler); }
}

/// Rust exception entry point called by the assembly common stub.
#[no_mangle]
extern "C" fn fjord_exception_entry(frame: &ExceptionFrame) -> ! {
    let vector = frame.vector as usize;
    let name = EXCEPTION_NAMES.get(vector).copied().unwrap_or("unknown exception");

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
        unsafe { asm!("cli; hlt", options(nomem, nostack, preserves_flags)); }
    }
}

global_asm!(r#"
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
"#);
