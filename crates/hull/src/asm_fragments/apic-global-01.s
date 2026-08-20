
.global fjord_timer_isr
fjord_timer_isr:
    push 0x40
    jmp fjord_irq_common

.global fjord_spurious_isr
fjord_spurious_isr:
    iretq

.global fjord_irq_common
fjord_irq_common:
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

    sub rsp, 8
    lea rdi, [rsp + 8]
    call fjord_irq_dispatch
    add rsp, 8

    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rsi
    pop rdi
    pop rbp
    pop rdx
    pop rcx
    pop rbx
    pop rax

    add rsp, 8
    iretq
