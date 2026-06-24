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
 * The image boots via PVH: the `.note.Xen` ELF note advertises a 32-bit entry
 * point. It must be reachable through a PT_NOTE program header *and* present
 * in a loadable segment, so it is placed first and assigned to both the `text`
 * load segment and the `note` segment.
 *
 * The image is linked at the conventional 1 MiB mark. The real higher-half
 * mapping arrives with the MMU work later in Phase 1. */

ENTRY(_start)

PHDRS {
    text   PT_LOAD FLAGS(5);   /* R E */
    rodata PT_LOAD FLAGS(4);   /* R   */
    data   PT_LOAD FLAGS(6);   /* R W */
    note   PT_NOTE FLAGS(4);   /* R   */
}

SECTIONS {
    . = 1M;

    .note.Xen : ALIGN(4) {
        KEEP(*(.note.Xen))
    } :text :note

    .text : ALIGN(16) {
        *(.text._start)
        *(.text .text.*)
    } :text

    . = ALIGN(4K);
    .rodata : {
        *(.rodata .rodata.*)
    } :rodata

    . = ALIGN(4K);
    .data : {
        *(.data .data.*)
    } :data

    .bss : ALIGN(4K) {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    /* End of the loaded kernel image; the early frame allocator never hands
       out physical frames below this watermark. */
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
