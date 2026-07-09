// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! Service supervision: start, watch, restart-with-backoff, dependency order.

/// Core service order. Dependencies flow left-to-right.
pub const BOOT_ORDER: &[ServiceKind] = &[
    ServiceKind::Timed,
    ServiceKind::Cryptd,
    ServiceKind::Storaged,
    ServiceKind::Vfs,
    ServiceKind::Netd,
];

/// Built-in service names supervised by Helm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceKind {
    /// Trusted time and counters.
    Timed,
    /// Key custody and crypto operations.
    Cryptd,
    /// Block/object storage.
    Storaged,
    /// Virtual filesystem.
    Vfs,
    /// Network stack.
    Netd,
}

/// Mutable health state for a service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceState {
    /// Service kind.
    pub kind: ServiceKind,
    /// Number of crashes since last healthy window.
    pub crashes: u32,
    /// Monotonic tick before which restart is refused.
    pub restart_after: u64,
    /// Whether this service is quarantined.
    pub quarantined: bool,
}

impl ServiceState {
    /// Creates a clean service state.
    #[must_use]
    pub const fn new(kind: ServiceKind) -> Self {
        Self {
            kind,
            crashes: 0,
            restart_after: 0,
            quarantined: false,
        }
    }

    /// Records a crash and computes exponential restart backoff.
    pub fn record_crash(&mut self, now: u64) {
        self.crashes = self.crashes.saturating_add(1);
        let shift = self.crashes.min(10);
        let backoff = 1u64 << shift;
        self.restart_after = now.saturating_add(backoff);
        if self.crashes >= 5 {
            self.quarantined = true;
        }
    }

    /// Records a healthy service window.
    pub fn record_healthy(&mut self) {
        self.crashes = 0;
        self.restart_after = 0;
        self.quarantined = false;
    }

    /// Whether Helm may attempt a restart at `now`.
    #[must_use]
    pub const fn can_restart(self, now: u64) -> bool {
        !self.quarantined && now >= self.restart_after
    }
}
