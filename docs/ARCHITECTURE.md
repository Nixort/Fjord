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

Helm is the first userspace task and holds the root CSpace. It:

- starts and supervises the core services;
- **verifies every `Cask`** before execution: signature (authenticity),
  BLAKE3 Merkle root (integrity), license budget (authorization), and a
  `Logbook` inclusion proof (transparency);
- computes the effective capability set as `manifest ∩ license ∩ delegated`.

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
