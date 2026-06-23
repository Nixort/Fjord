// Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
//
// License: GNU General Public License v3
// You can find the license file in the project root.
//
// Fjord OS — version 0.0.1
// The code was written for Fjord.
// 23 june 2026

//! Early debug console: 16550 UART (x86_64 COM1) / PL011 (aarch64, TODO).
//!
//! This is the *earliest* output path in the system — it must work before the
//! heap, before paging, and from inside the panic handler. It is therefore
//! deliberately tiny, blocking, and lock-free (valid only on the boot CPU
//! during early bring-up; real locking arrives with SMP in Phase 2).
//!
//! See `docs/ARCHITECTURE.md` §2 and ROADMAP Phase 1.

use core::fmt::{self, Write};

#[cfg(target_arch = "x86_64")]
mod imp {
    use core::arch::asm;

    /// COM1 base I/O port on PC-compatible x86_64 platforms.
    const COM1: u16 = 0x3F8;

    // 16550 register offsets from the port base.
    const DATA: u16 = 0; // RBR/THR, or divisor low when DLAB=1
    const IER: u16 = 1; // interrupt enable, or divisor high when DLAB=1
    const FCR: u16 = 2; // FIFO control (write-only)
    const LCR: u16 = 3; // line control
    const MCR: u16 = 4; // modem control
    const LSR: u16 = 5; // line status

    const LSR_THR_EMPTY: u8 = 1 << 5;

    /// Write a byte to an I/O port.
    ///
    /// # Safety
    /// `port` must be a valid, side-effect-safe I/O port for this platform.
    unsafe fn outb(port: u16, value: u8) {
        // SAFETY: `out` to a known UART port; caller upholds port validity.
        unsafe {
            asm!("out dx, al", in("dx") port, in("al") value,
                 options(nomem, nostack, preserves_flags));
        }
    }

    /// Read a byte from an I/O port.
    ///
    /// # Safety
    /// See [`outb`].
    unsafe fn inb(port: u16) -> u8 {
        let value: u8;
        // SAFETY: `in` from a known UART port; caller upholds port validity.
        unsafe {
            asm!("in al, dx", out("al") value, in("dx") port,
                 options(nomem, nostack, preserves_flags));
        }
        value
    }

    /// Initialise COM1: 38400 baud, 8N1, FIFOs on, interrupts masked.
    pub fn init() {
        // SAFETY: standard 16550 init sequence on the fixed COM1 port set.
        unsafe {
            outb(COM1 + IER, 0x00); // disable interrupts
            outb(COM1 + LCR, 0x80); // enable DLAB to program the divisor
            outb(COM1 + DATA, 0x03); // divisor low: 115200 / 3 = 38400 baud
            outb(COM1 + IER, 0x00); // divisor high
            outb(COM1 + LCR, 0x03); // DLAB off; 8 data bits, no parity, 1 stop
            outb(COM1 + FCR, 0xC7); // enable + clear FIFOs, 14-byte threshold
            outb(COM1 + MCR, 0x0B); // RTS + DTR + OUT2 (OUT2 gates IRQ line)
        }
    }

    fn can_send() -> bool {
        // SAFETY: reading the line-status register has no side effects.
        unsafe { inb(COM1 + LSR) & LSR_THR_EMPTY != 0 }
    }

    /// Transmit one byte, blocking until the holding register is free.
    pub fn put(byte: u8) {
        while !can_send() {
            core::hint::spin_loop();
        }
        // SAFETY: THR is empty; writing DATA enqueues the byte for transmit.
        unsafe { outb(COM1 + DATA, byte) }
    }
}

#[cfg(not(target_arch = "x86_64"))]
mod imp {
    //! Portable fallback. aarch64 PL011 support is tracked in ROADMAP Phase 1.

    /// No-op until a real driver exists for this architecture.
    pub fn init() {}

    /// TODO(hull): drive the PL011 UART on aarch64 (QEMU `virt` @ 0x0900_0000).
    pub fn put(_byte: u8) {}
}

/// A zero-sized handle to the early serial console.
///
/// Obtain an initialised handle with [`Serial::init`]. It implements
/// [`core::fmt::Write`], so the [`crate::kprint!`] / [`crate::kprintln!`]
/// macros and the kernel panic handler can format through it.
pub struct Serial;

impl Serial {
    /// Initialise the platform serial device and return a writer handle.
    ///
    /// Idempotent: safe to call again from the panic handler.
    pub fn init() -> Self {
        imp::init();
        Serial
    }

    /// Emit one byte, expanding `\n` to CRLF for terminal friendliness.
    pub fn write_byte(&mut self, byte: u8) {
        if byte == b'\n' {
            imp::put(b'\r');
        }
        imp::put(byte);
    }
}

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &byte in s.as_bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}

/// Backing function for [`crate::kprint!`]; not part of the stable API.
#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    // Early boot is single-threaded; a fresh ZST handle per call is fine until
    // we have a Keel-managed lock post-SMP (TODO(hull)).
    let mut serial = Serial;
    let _ = serial.write_fmt(args);
}

/// Print to the early serial console (no trailing newline).
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => ($crate::serial::_print(::core::format_args!($($arg)*)));
}

/// Print a line to the early serial console.
#[macro_export]
macro_rules! kprintln {
    () => ($crate::kprint!("\n"));
    ($($arg:tt)*) => ($crate::kprint!("{}\n", ::core::format_args!($($arg)*)));
}
