
.global fjord_ctx_switch
fjord_ctx_switch:
    mov x2, sp
    str x2,  [x0, #0x00]
    stp x19, x20, [x0, #0x08]
    stp x21, x22, [x0, #0x18]
    stp x23, x24, [x0, #0x28]
    stp x25, x26, [x0, #0x38]
    stp x27, x28, [x0, #0x48]
    stp x29, x30, [x0, #0x58]
    ldr x2,  [x1, #0x00]
    mov sp, x2
    ldp x19, x20, [x1, #0x08]
    ldp x21, x22, [x1, #0x18]
    ldp x23, x24, [x1, #0x28]
    ldp x25, x26, [x1, #0x38]
    ldp x27, x28, [x1, #0x48]
    ldp x29, x30, [x1, #0x58]
    ret
