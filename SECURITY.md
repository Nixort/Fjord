# Security Policy

## Reporting a vulnerability

Email nixort@proton.me with details and, if possible, a reproducer. Please do
**not** open public issues for security problems. We aim to acknowledge within
72 hours and to ship a coordinated fix with a Logbook-recorded advisory.

## Trusted Computing Base (TCB)

The fully trusted set is intentionally tiny:

- `anchor` (secure boot / DICE)
- `keel` (microkernel)
- the crypto core used by `cryptd` / `brine` / `cask`

Everything else is deprivileged and capability-confined. Bugs outside the TCB
should be containable by the capability model; report them anyway.

## Threat model

In scope: malicious or buggy userspace, executable substitution/modification,
rollback attacks, signing-key or publisher compromise (mitigated by Logbook
transparency + revocation), evil-maid and cold-boot attacks.

Out of scope: hardware backdoors, side-channels below the HAL, and a fully
compromised hardware root of trust.

## Cryptography

- Signatures: Ed25519 + ML-DSA (Dilithium) hybrid.
- Hash / integrity: BLAKE3 Merkle trees.
- AEAD: XChaCha20-Poly1305 (SW) / AES-256-GCM (AES-NI).
- Password KDF: Argon2id. Key encapsulation (optional PQ): X25519 + ML-KEM.

Never roll your own primitive; constant-time implementations only.
