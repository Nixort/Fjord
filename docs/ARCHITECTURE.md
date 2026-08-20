# Fjord Architecture

This document describes how the whole OS works. It is the canonical companion
to the source skeleton; every crate links back to a section here.

## 0. Design principles

1. **Minimal TCB.** Only `Anchor`, `Keel` and the crypto core are fully
   trusted. Everything else is a deprivileged, capability-confined component.
2. **No ambient authority.** There is no global root. A component can only do
   what its capabilities permit. Privilege is an explicit, unforgeable token.
3. **Language-level isolation.** Rust's ownership model is the first isolation
   boundary; hardware (page tables, MMU, optionally CHERI) is the second.
4. **End-to-end chain of trust.** firmware -> kernel -> services -> a specific
   executable -> a specific operation, with no gaps.
5. **Verifiability over features.** Prefer designs amenable to formal proof
   (seL4-style) and reproducible builds.

## 1. Keel — the microkernel

Keel is a capability microkernel (seL4 lineage). It implements only:

- **Capabilities (CSpace).** Every kernel object (page, endpoint, thread,
  IRQ) is named by an unforgeable capability. Authority is delegated by
  granting/minting derived capabilities.
- **Address spaces (VSpace).** Page-table management; W^X enforced.
- **IPC.** Synchronous endpoints (fast-path migrating-thread calls),
  asynchronous notifications, and shared-memory rings (`vmrings`).
- **Tide scheduler.** MCS-style scheduling contexts (budget + period) so CPU
  time is itself a capability — no unbounded priority inversion.

Everything else (drivers, FS, network, paging policy) lives in userspace.

**Implementation status (v0.0.2).** **Phase 2 is complete.** All six
mechanisms are heap-free and use caller-owned storage: `cap` (CSpace), `cdt`
(derivation/revocation), `vspace` (W^X map/translate/unmap), `untyped`, `ipc`
(synchronous endpoints, notifications and `vmring`), and `tide` (budgeted
priority scheduling). Their live boot integration is proven on x86_64 and
aarch64. Keel retypes bootstrap objects from real untyped RAM, clones the active
kernel root for each task VSpace, performs verified CR3/TTBR0 activate/restore
handoffs, and maintains TCB lifecycle plus scheduling-context ownership.

The Phase 2 exit proof creates two separate unprivileged VSpaces. Tide dispatches
the receiver, then the sender, then the awakened receiver; task-aware endpoint
IPC performs the receiver `Running → Blocked → Ready` transition; and the saved
user frame resumes with the transmitted word. QEMU smoke requires
`PHASE2: IPC_ROUNDTRIP PASS` on both targets. Tide uses bounded FIFO queues per
priority and a 256-bit ready bitmap, so highest-priority selection does not scan
the full admitted-task table. The Cask integrity MVP is also present (`crates/cask`; see §5).

## 2. Hull — hardware abstraction layer

Thin, mostly-safe wrappers over arch + platform: CPU init, MMU, interrupts,
timers, DMA-safe buffers, VirtIO transports. Per-arch backends behind one API
so the rest of the system is portable across x86_64 and aarch64.

## 3. Anchor — secure boot + DICE

Measured boot: each stage hashes the next into TPM PCRs and derives a
layered DICE identity (CDI). Keys (including Brine volume keys) are *sealed*
to the expected measurements, so a tampered boot chain cannot unseal them.
This is the root of the end-to-end chain of trust.

## 4. Helm — supervisor + Cask verifier

Helm is the policy-facing userspace supervisor and will hold the root CSpace.
Its target contract is to start core services, compute
`manifest ∩ license ∩ delegated`, and gate every Cask on authenticity,
integrity, authorization and transparency. Current code exposes a fail-closed
preflight: Cask integrity is verified, a detached signature block is required,
and a Logbook inclusion proof must be anchored by a caller-provided checkpoint
trust verifier. Full Cask signatures, license budgets, and the root-task service
manager remain Phase 3 work.

## 5. Cask — the executable format

A `.cask` is a tamper-evident container (see `crates/cask`):

- **Integrity:** content is a BLAKE3 Merkle tree; pages are verified lazily on
  fault (fs-verity style). W^X always.
- **Authenticity:** detached signatures (Ed25519 + ML-DSA hybrid) over the
  Merkle root and the `Lading` manifest. Trust anchors are capabilities.
- **Authorization:** the embedded license is a signed capability budget; Helm
  enforces it.
- **Anti-rollback:** a monotonic version counter checked against TPM NV / RPMB.
- **Transparency:** the signature must appear in `Logbook` with an inclusion
  proof, enabling detection and revocation.

**Implementation status (v0.0.2).** The integrity half is in tree and heap-free:
a from-scratch BLAKE3 (`cask::blake3`, pinned to upstream known-answer vectors),
an fs-verity-style Merkle tree whose root is sealed in the header
(`cask::merkle`), and a zero-copy, strictly bounds-checked parser
(`cask::format`). The eager verifier streams the Merkle root using `O(log
page_count)` hashes instead of materialising every level; the loader's lazy
`verify_page` remains allocation-free on fault. Authenticity (hybrid
signatures), anti-rollback, and license policy remain Phase 3 work. Logbook
inclusion now rejects unauthenticated checkpoints through a mandatory,
domain-separated trust-anchor interface, but the real Anchor/HSM-backed
Ed25519/ML-DSA verifier and append-only log service are intentionally still
unimplemented.

## 6. Brine — disk encryption

Authenticated encryption for storage: AEAD (XChaCha20-Poly1305, or AES-256 on
AES-NI) for confidentiality, the FS BLAKE3 Merkle tree for integrity, in a
single pass. Envelope key hierarchy (DEK -> VK -> KEK) gives instant rekey and
crypto-erase. VKs are sealed to the Anchor/TPM measurements. See
`crates/brine` and ARCHITECTURE §3.

## 7. Harbormaster — authorization

Bridges human/device authentication and the object-capability model. MFA
(FIDO2 passkey, Argon2id passphrase, on-chip biometric, device attestation)
establishes a principal; Harbormaster unseals Brine and mints a scoped
session CSpace with time-boxed capability leases. Step-up auth and continuous
attestation for sensitive operations.

## 8. Services

Userspace daemons over `fjord-rt`:
- **cryptd** — key custody; apps get operations, never raw keys.
- **storaged** — block + capability-addressed object storage (CoW).
- **vfs** — virtual filesystem and namespacing.
- **netd** — userspace TCP/IP (smoltcp lineage) over VirtIO.
- **timed** — trusted time and monotonic counters (anti-rollback).

## 9. Userspace runtime

`fjord-rt` is an async executor mapping futures onto Keel IPC + notifications.
`libfjord` exposes typed, capability-checked syscall bindings.

## 10. Build & supply chain (Shipwright)

Reproducible builds; PGO assigns optimization tiers per Cell; `Shipwright`
seals the resulting `.cask`, records provenance (SLSA/in-toto) and submits the
signature to `Logbook`.

## 11. Threat model (summary)

In scope: malicious/buggy userspace, file substitution/modification, rollback,
key/publisher compromise (mitigated by transparency + revocation), evil-maid,
cold-boot. Out of scope: hardware backdoors, leaks below the HAL, a fully
compromised root of trust.
