// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Cask launch path: verify -> build CSpace -> map -> start.
//!
//! This slice implements the fail-closed preflight policy. It verifies Cask
//! integrity, requires Logbook inclusion, and returns a launch plan. Actual
//! VSpace construction and task start remain Keel/Hull integration work.

use cask::format::Cask;
use cask::{verify, CaskError};
use logbook::{self, Checkpoint, InclusionProof, LogbookError, Revocation};

/// Preflight output consumed by the supervisor before it asks Keel to map/start.
pub struct LaunchPlan<'a> {
    /// Parsed and integrity-verified Cask view.
    pub cask: Cask<'a>,
    /// Number of body pages to map lazily.
    pub page_count: u64,
}

/// Why Helm refused to launch a Cask.
#[derive(Debug)]
pub enum LaunchError {
    /// Container parse or integrity check failed.
    Cask(CaskError),
    /// Transparency inclusion or revocation check failed.
    Logbook(LogbookError),
    /// Detached signature block is mandatory for executable Casks.
    MissingSignature,
}

impl From<CaskError> for LaunchError {
    fn from(err: CaskError) -> Self {
        Self::Cask(err)
    }
}

impl From<LogbookError> for LaunchError {
    fn from(err: LogbookError) -> Self {
        Self::Logbook(err)
    }
}

/// Verify and plan the launch of a Cask.
pub fn launch_cask<'a>(
    image: &'a [u8],
    checkpoint: &Checkpoint,
    proof: &InclusionProof,
    revocations: &[Revocation],
) -> Result<LaunchPlan<'a>, LaunchError> {
    let cask = verify::open_and_verify(image)?;
    let signature = cask.signature_bytes();
    if signature.is_empty() {
        return Err(LaunchError::MissingSignature);
    }
    let leaf = logbook::signature_leaf(signature);
    logbook::verify_inclusion(&leaf, checkpoint, proof, revocations)?;
    let page_count = cask.page_count();
    Ok(LaunchPlan { cask, page_count })
}
