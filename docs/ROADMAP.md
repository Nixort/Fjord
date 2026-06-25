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
- [x] GPL-3.0-or-later licensing
- [x] ARCHITECTURE + ROADMAP + CONTRIBUTING + SECURITY docs
- [ ] `shipwright` host orchestrator: `build`, `check`, `fmt`, `clippy`, `qemu`
- [ ] CI matrix (x86_64-unknown-none, aarch64-unknown-none) 🟡
- [ ] Reproducible-build harness (SOURCE_DATE_EPOCH, locked deps) 🟡
- [ ] `no_std` test harness running under QEMU

## Phase 1 — Boot & Hull  (🟡)

**Goal.** Boot to a Rust `kmain` on real-ish hardware (QEMU virt) on both archs.

**Exit criteria.** Serial "hello from Keel" on x86_64 + aarch64 under QEMU.

- [x] `Hull`: CPU bring-up, GDT/TSS/IDT + CPU exceptions (x86_64); aarch64 EL1 vector table + synchronous-exception dispatcher (ESR/FAR decode)
- [x] `Hull`: physical memory map discovery (PVH `hvm_start_info` on x86_64, flattened device tree `/memory` on aarch64), early bump frame allocator
- [x] `Hull`: MMU enable, per-section W^X page attributes (4 KiB kernel image + 2 MiB identity)
- [ ] `Hull`: higher-half kernel relocation
- [x] `Hull`: x86_64 local APIC + periodic timer interrupt (IDT gate → ISR → EOI → iretq)
- [x] `Hull`: aarch64 GIC v2 + ARM generic timer (PPI 30, EL1 IRQ path)
- [x] `Hull`: 16550 UART serial driver (x86_64) + `kprintln!`; PL011 (aarch64, QEMU `virt`)
- [x] `boot` crate: freestanding `_start` + boot stack -> `keel::kmain` (x86_64 PVH)
- [x] `boot` crate: aarch64 `_start` shim (EL2→EL1, `.bss` clear, `VBAR_EL1`) + QEMU `virt` link script
- [x] `keel::kmain` boot banner over the early serial console
- [x] Panic handler over serial (backtrace + early `alloc` bump->buddy pending)
- [x] Loader handoff: Multiboot1 + 32→64-bit long-mode trampoline; boots under `qemu -kernel` (UEFI/`limine` + memory map later) 🟡

## Phase 2 — Keel microkernel core  (🟡)

**Goal.** Capabilities, address spaces, IPC, and the Tide scheduler.

**Exit criteria.** Two userspace tasks exchange a message over an endpoint and
are scheduled by budget; a capability cannot be forged or escalated.

**Status.** The six core mechanisms now exist as heap-free, caller-owned-storage
models, each exercised by a boot-time self-test (green on x86_64 + aarch64). What
remains is *integration*: fusing the models into live kernel objects retyped from
untyped memory, wiring VSpace to the Hull page-table mapper, and a real context
switch driven by the timer IRQ.

- [x] CSpace: capability tables, derivation, revocation tree (`cap` + `cdt`, self-tested)
- [x] VSpace: page-table abstraction, map/unmap, W^X invariants (`vspace`, self-tested)
- [x] Untyped memory + retype model (seL4-style object allocation) (`untyped`, self-tested)
- [x] IPC: synchronous endpoints — rendezvous queue (`ipc`, self-tested; fast-path migrating-thread call still pending)
- [x] IPC: async notifications + `vmring` shared-memory rings (`ipc`, self-tested)
- [x] Tide: MCS scheduling contexts (budget/period), priorities (`tide`, self-tested)
- [ ] CTE integration: fuse cap/cdt/mapping into real CSpace slots retyped from untyped 🟡
- [x] VSpace ↔ Hull: `HwVSpace` fuses the bookkeeping `VSpace` with `hull`’s `Mapper` so `map`/`unmap` write real 4 KiB hardware leaves (W^X enforced); exercised on an inactive scratch address space — live-regime install (TTBR0/CR3 handoff) still pending ✅
- [x] Tide context switch: `hull::context` saves/restores callee-saved CPU state and resumes a fresh context on its own stack, proven by a cooperative round-trip self-test (`tide::ctx_selftest`, both arches) ✅
- [x] Tide preemptive switch: the platform timer ISR alone interleaves two non-cooperative worker contexts via a Hull tick hook (`hull::sched_hook`) that Keel registers, proven by `tide::preempt_selftest` (both arches) ✅
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
