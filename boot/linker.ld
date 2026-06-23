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
 * The image is linked at the conventional 1 MiB mark. The real higher-half
 * mapping and a proper memory layout arrive with the MMU work in Phase 1;
 * this is just enough to produce a coherent ELF with a known entry point. */

ENTRY(_start)

SECTIONS {
    . = 1M;

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
