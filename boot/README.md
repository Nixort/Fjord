# `boot/` — boot artifacts & kernel binary

This directory holds the bare-metal build inputs for the freestanding kernel
image. The image itself is the `boot` crate (`crates/boot`), which produces the
`fjord-kernel` ELF.

## Contents

| File | Purpose |
|------|---------|
| `x86_64-fjord.json` | Custom rustc target spec (no_std, soft-float, `panic=abort`, `rust-lld`). |
| `linker.ld` | Link script: `ENTRY(_start)`, image based at 1 MiB. |

## Building the kernel ELF

```sh
# nightly toolchain with rust-src is required (build-std).
cargo build -p boot --target boot/x86_64-fjord.json
```

The linker script is passed via `crates/boot/build.rs` (emits
`-Clink-arg=-T.../boot/linker.ld`).

## Status — ROADMAP Phase 1

- [x] Target spec + link script
- [x] Freestanding binary with `_start`, a boot stack, and a panic handler
      that transfers control to `keel::kmain`
- [ ] **Loader handoff (next sub-step).** `_start` currently assumes it is
      entered in 64-bit long mode by an external loader. The next commit wires
      a real loader path — most likely the `bootloader`/`limine` crate (gives
      us long-mode entry, a memory map, and a framebuffer) — so the image
      becomes directly bootable under `qemu-system-x86_64`.
- [ ] aarch64 entry shim (QEMU `virt`) + PL011 console.
