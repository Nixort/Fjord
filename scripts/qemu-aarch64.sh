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
# Build the aarch64 kernel, wrap it as a Linux arm64 Image (flat binary), and
# boot it under QEMU `virt`. The Image header makes QEMU pass the DTB pointer
# in x0; a bare ELF would not. Extra args are forwarded to QEMU (e.g. -m 512).
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

TARGET=aarch64-unknown-none-softfloat
ELF="target/${TARGET}/debug/fjord-kernel"
IMG="${ELF}.img"

cargo build -p boot --target "${TARGET}"

# llvm-objcopy ships with the `llvm-tools-preview` rustup component.
HOST="$(rustc -vV | sed -n 's/^host: //p')"
OBJCOPY="$(rustc --print sysroot)/lib/rustlib/${HOST}/bin/llvm-objcopy"
if [ ! -x "${OBJCOPY}" ]; then
  echo "llvm-objcopy not found at ${OBJCOPY}" >&2
  echo "Install it with: rustup component add llvm-tools-preview" >&2
  exit 1
fi

"${OBJCOPY}" -O binary "${ELF}" "${IMG}"
echo "built ${IMG} ($(stat -c %s "${IMG}") bytes)"

exec qemu-system-aarch64 \
  -M virt -cpu cortex-a72 \
  -kernel "${IMG}" \
  -serial stdio -display none \
  "$@"
