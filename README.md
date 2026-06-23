# Fjord — Fortified Joint Operating Rust Daemon

> A security-first, fully independent operating system written entirely in Rust.

Fjord is a capability-based microkernel OS. Its design goal is an *ideally
balanced* system: a tiny verifiable trusted computing base (TCB), strong
language-level isolation from Rust, and an end-to-end chain of trust that runs
`firmware -> kernel -> services -> a specific executable -> a specific operation`.

This repository is an **early-stage skeleton**. Almost everything is a
documented stub marked with `TODO(...)`. Read `docs/ARCHITECTURE.md` for the
full design and `docs/ROADMAP.md` for the phased delivery plan.

## Naming (nautical codenames)

| Codename       | Crate                     | Role |
|----------------|---------------------------|------|
| **Keel**       | `crates/keel`             | Microkernel: capabilities, IPC, memory, the Tide scheduler |
| **Hull**       | `crates/hull`             | Hardware Abstraction Layer (HAL) |
| **Anchor**     | `crates/anchor`           | Secure boot + DICE measured-boot root of trust |
| **Helm**       | `crates/helm`             | Root supervisor + Cask verifier (the policy brain) |
| **Cask**       | `crates/cask`             | `.cask` tamper-evident executable container format |
| **Lading**     | `crates/lading`           | Signed bill-of-lading manifest |
| **Brine**      | `crates/brine`            | Full-disk / per-object authenticated encryption |
| **Harbormaster**| `crates/harbormaster`    | Authentication + authorization (identity -> capability) |
| **Logbook**    | `crates/logbook`          | Append-only transparency log + inclusion proofs |
| **Shipwright** | `xtask/shipwright`        | Host build orchestrator (PGO tiers, seals Cask) |
| `fjord-rt`     | `crates/fjord-rt`         | Async runtime for userspace services |
| `libfjord`     | `crates/libfjord`         | Userspace syscall + capability bindings |
| services       | `crates/services/*`       | cryptd, storaged, vfs, netd, timed |

## How it boots (one-paragraph mental model)

`Anchor` measures each stage into the TPM (DICE) and only releases keys when the
chain is intact. It hands control to `Keel`, the microkernel, which sets up
capabilities, address spaces and the `Tide` scheduler. `Keel` starts `Helm`,
the root supervisor, which holds the initial capability set and verifies every
`Cask` executable (signature + Merkle integrity + license budget + `Logbook`
inclusion proof) before launching it. User-facing login goes through
`Harbormaster`, which authenticates the principal, unseals the `Brine` volume
keys, and mints a scoped capability set for the session.

## Building

```sh
rustup toolchain install nightly        # toolchain is pinned in rust-toolchain.toml
cargo shipwright -- build --profile dev # host orchestrator drives the build
```

Kernel crates are `#![no_std]` and target `x86_64-unknown-none` /
`aarch64-unknown-none`. See `CONTRIBUTING.md` and `SECURITY.md`.

## License

Dual-licensed under MIT or Apache-2.0 at your option.
