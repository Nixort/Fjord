/* Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
 *
 * License: GNU General Public License v3
 * You can find the license file in the project root.
 *
 * Fjord OS — version 0.0.1
 * The code was written for Fjord.
 * 23 june 2026
 */

/* Minimal kernel link script for the aarch64 `boot` binary on QEMU `virt`.
 *
 * QEMU's `virt` machine places RAM at 0x4000_0000. The image is linked at
 * 0x4008_0000 (the conventional 512 KiB kernel offset) and QEMU's `-kernel`
 * loader enters the ELF at `_start` directly, so no boot note is required.
 *
 * Section boundaries are exported as 4 KiB-aligned symbols so a later MMU slice
 * can map each segment with exact W^X permissions. The boot stack lives in
 * `.bss` and is therefore covered by the early `.bss` clear in `boot-aarch64.s`. */

ENTRY(_start)

PHDRS {
    text   PT_LOAD FLAGS(5);   /* R E */
    rodata PT_LOAD FLAGS(4);   /* R   */
    data   PT_LOAD FLAGS(6);   /* R W */
}

SECTIONS {
    . = 0x40080000;
    . = ALIGN(4K);
    __text_start = .;

    .text : ALIGN(16) {
        KEEP(*(.text._start))
        KEEP(*(.text.vectors))
        *(.text .text.*)
    } :text

    . = ALIGN(4K);
    __text_end = .;
    __rodata_start = .;
    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    . = ALIGN(4K);
    __rodata_end = .;
    __data_start = .;
    .data : {
        *(.data .data.*)
    } :data

    .bss (NOLOAD) : ALIGN(16) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        __boot_stack_bottom = .;
        . += 0x10000;          /* 64 KiB boot stack */
        __boot_stack_top = .;
        . = ALIGN(4K);
        __bss_end = .;
    } :data

    /* End of the loaded kernel image; a later frame allocator never hands out
       physical frames below this watermark. */
    . = ALIGN(4K);
    __kernel_end = .;

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note.GNU-stack)
        *(.note.gnu.property)
        *(.note.gnu.build-id)
        *(.comment)
    }
}
