# `boot/` — boot artifacts & kernel binary

This directory holds the bare-metal build inputs for the freestanding kernel
image. The image itself is the `boot` crate (`crates/boot`), which produces the
`fjord-kernel` ELF.

## Contents

| File | Purpose |
|------|---------|
| `x86_64-fjord.json` | Custom rustc target spec (no_std, `panic=abort`, `rust-lld`). |
| `linker.ld` | Link script: `ENTRY(_start)`, PVH note + PT_NOTE phdr, image based at 1 MiB. |

## Boot protocol

The kernel boots via the **PVH** boot protocol. `crates/boot/src/boot.s` embeds
an ELF note (`XEN_ELFNOTE_PHYS32_ENTRY`) advertising a 32-bit entry point and
provides `_start`, which is entered in 32-bit protected mode. The shim:

1. sets up a temporary stack and stashes the multiboot info pointer;
2. identity-maps the low 1 GiB with 2 MiB pages;
3. enables PAE, SSE, long mode (`EFER.LME`) and paging;
4. far-returns into 64-bit code and calls `rust_entry` → `keel::kmain`.

This means the 64-bit ELF boots directly under `qemu-system-x86_64 -kernel`
(and Xen / cloud-hypervisor), with no external bootloader needed for
development. QEMU's built-in multiboot loader is intentionally not used: it
only accepts 32-bit ELF images, whereas the kernel is a 64-bit ELF.

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
- [x] PVH boot note + 32→64-bit long-mode trampoline (`boot.s`)
- [x] Freestanding binary that boots directly under QEMU and calls `keel::kmain`
- [ ] Parse the PVH `hvm_start_info` memory map and feed the early frame allocator.
- [ ] Higher-half kernel mapping (proper MMU module in Hull).
- [ ] UEFI / `limine` path for real hardware + a framebuffer.
- [ ] aarch64 entry shim (QEMU `virt`) + PL011 console.
