
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
