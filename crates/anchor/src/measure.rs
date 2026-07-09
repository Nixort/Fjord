// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! DICE measurement + layered Compound Device Identifier (CDI) derivation.

use cask::blake3;

/// 32-byte measurement digest.
pub type Digest = [u8; blake3::OUT_LEN];

const PCR_DOMAIN: &[u8] = b"fjord.anchor.pcr.v1";
const CDI_DOMAIN: &[u8] = b"fjord.anchor.cdi.v1";
const STAGE_DOMAIN: &[u8] = b"fjord.anchor.stage.v1";

/// Measurement of one boot stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StageMeasurement {
    /// Caller-defined stage identifier.
    pub stage_id: u64,
    /// BLAKE3 digest of the stage image under an Anchor domain separator.
    pub digest: Digest,
}

/// Hashes a boot stage image into a domain-separated measurement.
#[must_use]
pub fn measure_stage(stage_id: u64, image: &[u8]) -> StageMeasurement {
    let mut hasher = blake3::Hasher::new();
    hasher.update(STAGE_DOMAIN);
    hasher.update(&stage_id.to_le_bytes());
    hasher.update(image);
    StageMeasurement {
        stage_id,
        digest: hasher.finalize(),
    }
}

/// Extends a PCR-like digest with one stage measurement.
#[must_use]
pub fn extend_pcr(current: &Digest, measurement: &StageMeasurement) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(PCR_DOMAIN);
    hasher.update(current);
    hasher.update(&measurement.stage_id.to_le_bytes());
    hasher.update(&measurement.digest);
    hasher.finalize()
}

/// Derives the next DICE CDI from a parent CDI and a stage measurement.
#[must_use]
pub fn derive_cdi(parent_cdi: &Digest, measurement: &StageMeasurement) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(CDI_DOMAIN);
    hasher.update(parent_cdi);
    hasher.update(&measurement.stage_id.to_le_bytes());
    hasher.update(&measurement.digest);
    hasher.finalize()
}

/// Fixed-size measured-boot accumulator used before allocation exists.
#[derive(Clone, Copy, Debug)]
pub struct MeasurementChain {
    pcr: Digest,
    cdi: Digest,
}

impl MeasurementChain {
    /// Creates a chain from device-root PCR/CDI seeds.
    #[must_use]
    pub const fn new(root_pcr: Digest, root_cdi: Digest) -> Self {
        Self {
            pcr: root_pcr,
            cdi: root_cdi,
        }
    }

    /// Measures and extends one stage, returning its measurement.
    pub fn extend(&mut self, stage_id: u64, image: &[u8]) -> StageMeasurement {
        let measurement = measure_stage(stage_id, image);
        self.pcr = extend_pcr(&self.pcr, &measurement);
        self.cdi = derive_cdi(&self.cdi, &measurement);
        measurement
    }

    /// Current PCR-like accumulator.
    #[must_use]
    pub const fn pcr(&self) -> &Digest {
        &self.pcr
    }

    /// Current layered CDI.
    #[must_use]
    pub const fn cdi(&self) -> &Digest {
        &self.cdi
    }
}
