
.section .text.vectors, "ax"
.global el1_sync
el1_sync:
    mrs     x0, esr_el1
    mrs     x1, far_el1
    mrs     x2, elr_el1
    mrs     x3, spsr_el1
    b       fjord_aarch64_sync
