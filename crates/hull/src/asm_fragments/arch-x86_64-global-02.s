
.global fjord_user_run
fjord_user_run:
    // rdi = *mut UserFrame.
    // Save kernel callee-saved state + flags, stash rsp and the frame pointer.
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    pushfq
    mov [rip + FJORD_KERNEL_RSP], rsp
    mov [rip + FJORD_CURRENT_FRAME], rdi
    // Build the iretq frame for ring 3 (rax is a scratch; loaded for real below).
    push 0x33                 // user SS (GDT index 6 | RPL 3)
    mov rax, [rdi + 0x88]
    push rax                  // user RSP
    mov rax, [rdi + 0x80]
    push rax                  // RFLAGS
    push 0x2b                 // user CS (GDT index 5 | RPL 3)
    mov rax, [rdi + 0x78]
    push rax                  // user RIP
    // Load the user GPRs from the frame (rdi last, since it is the base).
    mov rax, [rdi + 0x00]
    mov rbx, [rdi + 0x08]
    mov rcx, [rdi + 0x10]
    mov rdx, [rdi + 0x18]
    mov rsi, [rdi + 0x20]
    mov rbp, [rdi + 0x30]
    mov r8,  [rdi + 0x38]
    mov r9,  [rdi + 0x40]
    mov r10, [rdi + 0x48]
    mov r11, [rdi + 0x50]
    mov r12, [rdi + 0x58]
    mov r13, [rdi + 0x60]
    mov r14, [rdi + 0x68]
    mov r15, [rdi + 0x70]
    mov rdi, [rdi + 0x28]
    iretq

.global fjord_user_syscall_isr
fjord_user_syscall_isr:
    // Entered from ring 3 via int 0x80 on the TSS.rsp0 stack. The CPU pushed
    // [rip, cs, rflags, user_rsp, ss]; save the full ring-3 state into the
    // current frame, then unwind back into the user_run caller.
    push rax                       // [rsp] = user rax; iret frame now at [rsp+8]
    mov rax, [rip + FJORD_CURRENT_FRAME]
    mov [rax + 0x08], rbx
    mov [rax + 0x10], rcx
    mov [rax + 0x18], rdx
    mov [rax + 0x20], rsi
    mov [rax + 0x28], rdi
    mov [rax + 0x30], rbp
    mov [rax + 0x38], r8
    mov [rax + 0x40], r9
    mov [rax + 0x48], r10
    mov [rax + 0x50], r11
    mov [rax + 0x58], r12
    mov [rax + 0x60], r13
    mov [rax + 0x68], r14
    mov [rax + 0x70], r15
    mov rcx, [rsp]                 // user rax
    mov [rax + 0x00], rcx
    mov rcx, [rsp + 0x08]          // rip
    mov [rax + 0x78], rcx
    mov rcx, [rsp + 0x18]          // rflags
    mov [rax + 0x80], rcx
    mov rcx, [rsp + 0x20]          // user rsp
    mov [rax + 0x88], rcx
    // Unwind to the kernel (mirror of fjord_user_run's prologue).
    mov rsp, [rip + FJORD_KERNEL_RSP]
    popfq
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret
