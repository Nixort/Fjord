// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.2
// The code was written for Fjord.
// 23 june 2026

//! # libfjord — userspace syscall + capability bindings
//!
//! Typed, capability-checked wrappers around Keel's IPC ABI. Application and
//! service code links this instead of issuing raw syscalls.
//! See `docs/ARCHITECTURE.md` §9.
#![no_std]
#![allow(dead_code)]
extern crate alloc;

/// A userspace handle to a capability held in the task's CSpace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cap(pub u32); // index into the CSpace

impl Cap {
    /// Returns whether the handle names the null/reserved slot.
    #[must_use]
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// Maximum inline words transported by the stable userspace ABI.
pub const MAX_MSG_WORDS: usize = 8;

/// Typed userspace IPC message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    /// Operation selector understood by the endpoint server.
    pub label: u64,
    words: [u64; MAX_MSG_WORDS],
    len: usize,
}

impl Message {
    /// Builds a bounded inline message; extra words are rejected by the caller.
    pub fn new(label: u64, words: &[u64]) -> Result<Self, InvokeError> {
        if words.len() > MAX_MSG_WORDS {
            return Err(InvokeError::MessageTooLarge);
        }
        let mut inline = [0u64; MAX_MSG_WORDS];
        for (dst, src) in inline.iter_mut().zip(words) {
            *dst = *src;
        }
        Ok(Self {
            label,
            words: inline,
            len: words.len(),
        })
    }

    /// Empty request with only a label.
    #[must_use]
    pub const fn empty(label: u64) -> Self {
        Self {
            label,
            words: [0; MAX_MSG_WORDS],
            len: 0,
        }
    }

    /// Number of valid inline words.
    #[must_use]
    pub const fn len(self) -> usize {
        self.len
    }

    /// Returns true when there are no inline words.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Gets one inline word.
    #[must_use]
    pub fn word(self, index: usize) -> Option<u64> {
        if index < self.len {
            Some(self.words[index])
        } else {
            None
        }
    }
}

/// Userspace IPC response.
pub type Response = Message;

/// Invocation errors visible to userspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvokeError {
    /// The null capability cannot be invoked.
    NullCap,
    /// The inline payload exceeded [`MAX_MSG_WORDS`].
    MessageTooLarge,
    /// The architecture syscall veneer has not been linked yet.
    BackendUnavailable,
}

/// Invoke an endpoint capability with an empty typed message.
pub fn invoke(cap: Cap) -> Result<Response, InvokeError> {
    invoke_raw(cap, Message::empty(0))
}

/// Invoke an endpoint capability with a typed message.
///
/// This is a fail-closed ABI wrapper: it validates the stable userspace shape
/// now, and returns [`InvokeError::BackendUnavailable`] until the per-arch
/// syscall veneer is wired to Keel. Callers therefore cannot accidentally treat
/// a missing kernel transport as success.
pub fn invoke_raw(cap: Cap, msg: Message) -> Result<Response, InvokeError> {
    if cap.is_null() {
        return Err(InvokeError::NullCap);
    }
    let _ = msg;
    Err(InvokeError::BackendUnavailable)
}
