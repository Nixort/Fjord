/* Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
 *
 * License: GNU General Public License v3
 * You can find the license file in the project root.
 *
 * Fjord OS — version 0.0.1
 * The code was written for Fjord.
 * 23 june 2026
 */

/* Minimal kernel link script for the x86_64 `boot` binary.
 *
 * The image is linked at the conventional 1 MiB mark. The Multiboot1 header
 * must live within the first 8 KiB of the file, so the `.boot` section is
 * placed first and KEPT (it is only referenced by the loader, never by Rust).
 * The real higher-half mapping arrives with the MMU work later in Phase 1. */

ENTRY(_start)

SECTIONS {
    . = 1M;

    .boot : ALIGN(8) {
        KEEP(*(.multiboot))
    }

    .text : ALIGN(4K) {
        *(.text._start)
        *(.text .text.*)
    }

    .rodata : ALIGN(4K) {
        *(.rodata .rodata.*)
    }

    .data : ALIGN(4K) {
        *(.data .data.*)
    }

    .bss : ALIGN(4K) {
        *(.bss .bss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.eh_frame)
        *(.note .note.*)
        *(.comment)
    }
}
