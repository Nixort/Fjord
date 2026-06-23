# Fjord Roadmap

Status legend: ✅ done · 🟡 in progress · ⬜ planned · 🔬 research

This roadmap is intentionally granular: each phase lists goals, exit criteria,
and concrete task checklists. Confidence markers: 🟢 certain · 🟡 ~80% · 🔴 uncertain.

---

## Phase 0 — Foundations & tooling  (🟡)

**Goal.** A reproducible build environment and an empty-but-coherent workspace.

**Exit criteria.** `cargo shipwright -- build` runs; CI lints + formats; every
crate compiles as a stub for both targets.

- [x] Virtual Cargo workspace + pinned nightly toolchain
- [x] Dual MIT/Apache-2.0 licensing
- [x] ARCHITECTURE + ROADMAP + CONTRIBUTING + SECURITY docs
- [ ] `shipwright` host orchestrator: `build`, `check`, `fmt`, `clippy`, `qemu`
- [ ] CI matrix (x86_64-unknown-none, aarch64-unknown-none) 🟡
- [ ] Reproducible-build harness (SOURCE_DATE_EPOCH, locked deps) 🟡
- [ ] `no_std` test harness running under QEMU

## Phase 1 — Boot & Hull  (⬜)

**Goal.** Boot to a Rust `kmain` on real-ish hardware (QEMU virt) on both archs.

**Exit criteria.** Serial "hello from Keel" on x86_64 + aarch64 under QEMU.

- [ ] `Hull`: CPU bring-up, GDT/IDT (x86_64), exception vectors (aarch64)
- [ ] `Hull`: physical memory map discovery, early frame allocator
- [ ] `Hull`: MMU enable, higher-half kernel, W^X page attributes
- [ ] `Hull`: timer + interrupt controller (APIC / GIC)
- [ ] `Hull`: 16550 / PL011 UART serial driver
- [ ] `bootloader` integration (UEFI) + handoff struct
- [ ] Panic handler, backtrace, early `alloc` (bump -> buddy)

## Phase 2 — Keel microkernel core  (⬜)

**Goal.** Capabilities, address spaces, IPC, and the Tide scheduler.

**Exit criteria.** Two userspace tasks exchange a message over an endpoint and
are scheduled by budget; a capability cannot be forged or escalated.

- [ ] CSpace: capability tables, derivation, revocation tree
- [ ] VSpace: page-table abstraction, map/unmap, W^X invariants
- [ ] Untyped memory + retype model (seL4-style object allocation)
- [ ] IPC: synchronous endpoints (fast-path migrating-thread call)
- [ ] IPC: async notifications + `vmring` shared-memory rings
- [ ] Tide: MCS scheduling contexts (budget/period), priorities
- [ ] IRQ delivery as capabilities to userspace drivers
- [ ] First userspace task launch from Keel
- [ ] `Cask` MVP: parse + BLAKE3 Merkle verify (loader path) 🟡

## Phase 3 — Trust, identity & encryption  (⬜)

**Goal.** The end-to-end chain of trust is real: measured boot, signed Casks,
encrypted disk, and authenticated login.

**Exit criteria.** A signed `.cask` that is absent from `Logbook`, rolled back,
or over-budget is refused; the disk is unreadable without successful MFA.

- [ ] `Anchor`: DICE measured boot, TPM PCR extend, layered CDI
- [ ] `Anchor`: key sealing/unsealing to measurements
- [ ] `Cask`: full seal/verify (Ed25519 + ML-DSA hybrid signatures)
- [ ] `Cask`: license-as-capability-budget; Helm enforces manifest ∩ license
- [ ] `Cask`: anti-rollback via monotonic counter (TPM NV / RPMB)
- [ ] `Logbook`: append-only log, inclusion proofs, revocation feed
- [ ] `Brine`: envelope keys (DEK/VK/KEK), AEAD data path
- [ ] `Brine`: crypto-erase, anti-rollback of ciphertext blocks
- [ ] `Harbormaster`: FIDO2/passkey + Argon2id + device attestation
- [ ] `Harbormaster`: unlock flow -> unseal VK -> mint session CSpace
- [ ] `Harbormaster`: capability leases, step-up auth, lockout counter
- [ ] `cryptd` + `timed` services backing the above

## Phase 4 — Storage, filesystem & networking  (⬜)

**Goal.** Persist data and talk to the network entirely from userspace.

**Exit criteria.** Mount an encrypted CoW volume, read/write files, complete a
TCP handshake, all as deprivileged services.

- [ ] `storaged`: VirtIO-blk driver, block cache, CoW extents
- [ ] `storaged`: capability-addressed object store + Merkle integrity
- [ ] `vfs`: namespacing, mount table, POSIX-ish file ops over IPC
- [ ] `netd`: VirtIO-net driver, smoltcp-lineage TCP/IP, sockets-as-capabilities
- [ ] Brine integrated at the storaged boundary (decrypt-on-fault)
- [ ] Crash-consistency / journaling tests

## Phase 5 — Userspace runtime & SDK  (⬜)

**Goal.** Make Fjord pleasant to develop for.

- [ ] `fjord-rt`: async executor over IPC + notifications, timers
- [ ] `libfjord`: typed capability-checked syscalls, error model
- [ ] Process model, service manager, dependency-ordered startup
- [ ] A minimal shell + a handful of demo apps shipped as Casks
- [ ] `Shipwright`: PGO tier assignment, provenance (SLSA/in-toto)

## Phase 6 — Assurance & hardening  (🔬)

**Goal.** Move from "works" to "trustworthy".

- [ ] Kani/Prusti/Creusot proofs for Keel capability + IPC invariants 🔴
- [ ] Miri + fuzzing (cargo-fuzz) for parsers (Cask, Lading, net)
- [ ] Constant-time review of all crypto; KAT vectors
- [ ] CHERI/Morello experiment for hardware capability backing 🔴
- [ ] Post-quantum migration hardening (ML-DSA, ML-KEM) 🟡
- [ ] Third-party security audit; reproducible-build attestation

## Cross-cutting tracks (continuous)

- **Docs:** keep ARCHITECTURE in sync with code; per-crate rustdoc.
- **Testing:** unit + QEMU integration + property tests per crate.
- **Supply chain:** `cargo deny`, SBOM, signed releases via Logbook.
- **Performance:** microbench IPC latency, AEAD throughput, fault path.
