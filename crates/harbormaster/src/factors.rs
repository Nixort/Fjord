//! Authentication factors. Policy requires N-of-M by sensitivity.
//!
//! TODO(harbormaster): implement verifiers; biometrics match on-chip only.
#[derive(Clone, Copy, Debug)]
pub enum FactorKind {
    /// FIDO2/WebAuthn passkey or hardware token (phishing-resistant).
    Passkey,
    /// Passphrase stretched with Argon2id.
    Passphrase,
    /// On-chip biometric match (template never leaves the secure element).
    Biometric,
    /// DICE/TPM device attestation (the device authenticates itself).
    DeviceAttestation,
}
