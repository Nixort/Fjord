// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

// Fjord boot shim.
//
// A Multiboot1-compliant bootloader (or `qemu-system-x86_64 -kernel`) loads
// this ELF and enters `_start` in 32-bit protected mode with paging off. The
// shim builds an identity-mapped page hierarchy for the low 1 GiB, switches
// the CPU into 64-bit long mode, then calls the Rust entry `rust_entry`.
//
// This toolchain assembles `global_asm!` in Intel syntax by default, so no
// `.intel_syntax` directive is used. Comments use `//`.

.set MB_MAGIC,    0x1BADB002
.set MB_FLAGS,    0x00000000
.set MB_CHECKSUM, -(MB_MAGIC + MB_FLAGS)

.section .multiboot, "a"
.align 4
    .long MB_MAGIC
    .long MB_FLAGS
    .long MB_CHECKSUM

.section .text._start, "ax"
.code32
.global _start
_start:
    cli
    lea esp, [boot_stack_top]

    // Preserve the multiboot info pointer (ebx) for the Rust side.
    mov [mb_info_ptr], ebx
    mov dword ptr [mb_info_ptr + 4], 0

    call zero_tables
    call setup_page_tables
    call enter_long_mode

    // Far-return into the 64-bit code segment (selector 0x08).
    lgdt [gdt64_pointer]
    push 0x08
    lea eax, [long_mode_start]
    push eax
    retf

// Zero PML4 and PDPT so unused entries are guaranteed not-present.
zero_tables:
    xor eax, eax
    lea edi, [pml4]
    mov ecx, 1024
    rep stosd
    lea edi, [pdpt]
    mov ecx, 1024
    rep stosd
    ret

// Identity-map the low 1 GiB using 2 MiB pages.
setup_page_tables:
    lea eax, [pdpt]
    or eax, 0x03                 // present | writable
    mov [pml4], eax
    mov dword ptr [pml4 + 4], 0

    lea eax, [pd]
    or eax, 0x03                 // present | writable
    mov [pdpt], eax
    mov dword ptr [pdpt + 4], 0

    xor ecx, ecx
fill_pd:
    mov esi, ecx
    shl esi, 21                  // ecx * 2 MiB
    or esi, 0x83                 // present | writable | huge
    mov [pd + ecx*8], esi
    mov dword ptr [pd + ecx*8 + 4], 0
    inc ecx
    cmp ecx, 512
    jne fill_pd
    ret

// Turn on PAE + SSE, set long-mode enable, then enable paging.
enter_long_mode:
    lea eax, [pml4]
    mov cr3, eax

    mov eax, cr4
    or eax, 0x20                 // CR4.PAE
    or eax, 0x600                // CR4.OSFXSR | CR4.OSXMMEXCPT (SSE)
    mov cr4, eax

    mov ecx, 0xC0000080          // IA32_EFER
    rdmsr
    or eax, 0x100                // EFER.LME
    wrmsr

    mov eax, cr0
    or eax, 0x80000000           // CR0.PG
    or eax, 0x2                  // CR0.MP
    and eax, 0xFFFFFFFB          // clear CR0.EM
    mov cr0, eax
    ret

.code64
long_mode_start:
    xor ax, ax
    mov ss, ax
    mov ds, ax
    mov es, ax
    mov fs, ax
    mov gs, ax

    lea rsp, [boot_stack_top]
    xor rbp, rbp
    mov rdi, [mb_info_ptr]       // first argument: multiboot info pointer
    call rust_entry
hang:
    cli
    hlt
    jmp hang

.section .rodata
.align 8
gdt64:
    .quad 0
    .quad 0x00209A0000000000     // 64-bit ring-0 code segment
gdt64_pointer:
    .word gdt64_pointer - gdt64 - 1
    .quad gdt64

.section .bss, "aw", @nobits
.align 8
mb_info_ptr:
    .skip 8
.align 4096
pml4:
    .skip 4096
pdpt:
    .skip 4096
pd:
    .skip 4096
.align 16
boot_stack_bottom:
    .skip 65536
boot_stack_top:
