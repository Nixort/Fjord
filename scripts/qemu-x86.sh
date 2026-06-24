#!/usr/bin/env bash
# Copyright Nixort & Itan Winter <https://github.com/Nixort/Fjord> 2026.
#
# License: GNU General Public License v3
# You can find the license file in the project root.
#
# Fjord OS — version 0.0.1
# The code was written for Fjord.
# 24 june 2026
#
# Build the x86_64 freestanding kernel ELF and boot it under QEMU via the PVH
# boot protocol (the .note.Xen ELF note advertises the 32-bit entry point, so
# `-kernel` loads the 64-bit ELF directly). Serial is routed to stdio and a
# guest-error log is written to target/qemu-fjord.log. Extra args are forwarded
# to QEMU (e.g. -m 512, -s -S for gdb). Mirror of scripts/qemu-aarch64.sh.
#
# This is the thin shell equivalent of `cargo shipwright -- qemu`; use whichever
# is handier. Both build the same ELF with the JSON target spec.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

TARGET_SPEC="boot/x86_64-fjord.json"
TARGET_TRIPLE="x86_64-fjord"
ELF="target/${TARGET_TRIPLE}/debug/fjord-kernel"

# -Zjson-target-spec is unstable, hence the pinned nightly in rust-toolchain.toml.
# build-std + the bare-metal target come from .cargo/config.toml.
cargo build -Zjson-target-spec -p boot --target "${TARGET_SPEC}"

if [ ! -f "${ELF}" ]; then
  echo "kernel ELF not found at ${ELF}" >&2
  exit 1
fi
echo "built ${ELF} ($(stat -c %s "${ELF}") bytes)"

if ! command -v qemu-system-x86_64 >/dev/null 2>&1; then
  echo "qemu-system-x86_64 not found in PATH" >&2
  echo "Install it with: sudo apt install qemu-system-x86 (or your distro equivalent)" >&2
  exit 1
fi

exec qemu-system-x86_64 \
  -kernel "${ELF}" \
  -serial stdio -display none \
  -no-reboot \
  -D target/qemu-fjord.log -d guest_errors \
  "$@"
