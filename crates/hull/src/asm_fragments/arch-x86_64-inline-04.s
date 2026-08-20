lgdt [{gdt}]
push {code}
lea rax, [rip + 2f]
push rax
retfq
2:
mov ax, {data}
mov ds, ax
mov es, ax
mov ss, ax
mov fs, ax
mov gs, ax
