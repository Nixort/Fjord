# `boot/` — boot artifacts & kernel binary

This directory holds the bare-metal build inputs for the freestanding kernel
image. The image itself is the `boot` crate (`crates/boot`), which produces the
`fjord-kernel` ELF.

## Contents

| File | Purpose |
|------|---------|
| `x86_64-fjord.json` | Custom rustc target spec (no_std, `panic=abort`, `rust-lld`). |
| `linker.ld` | Link script: `ENTRY(_start)`, Multiboot header first, image based at 1 MiB. |

## Boot protocol

The kernel is **Multiboot1**-compliant. `crates/boot/src/boot.s` embeds the
Multiboot header and provides `_start`, which is entered in 32-bit protected
mode. The shim:

1. sets up a temporary stack and stashes the multiboot info pointer;
2. identity-maps the low 1 GiB with 2 MiB pages;
3. enables PAE, SSE, long mode (`EFER.LME`) and paging;
4. far-returns into 64-bit code and calls `rust_entry` → `keel::kmain`.

This means the ELF boots directly under `qemu-system-x86_64 -kernel`, with no
external bootloader needed for development.

## Building & booting

```sh
# nightly toolchain with rust-src is required (build-std).
cargo shipwright -- build        # build the kernel ELF
cargo shipwright -- qemu         # build, then boot in QEMU (serial on stdio)
```

The linker script is passed via `crates/boot/build.rs` (emits
`-Clink-arg=-T.../boot/linker.ld`).

## Status — ROADMAP Phase 1

- [x] Target spec + link script
- [x] Multiboot1 header + 32→64-bit long-mode trampoline (`boot.s`)
- [x] Freestanding binary that boots directly under QEMU and calls `keel::kmain`
- [ ] Parse the Multiboot memory map and feed the early frame allocator.
- [ ] Higher-half kernel mapping (proper MMU module in Hull).
- [ ] UEFI / `limine` path for real hardware + a framebuffer.
- [ ] aarch64 entry shim (QEMU `virt`) + PL011 console.
