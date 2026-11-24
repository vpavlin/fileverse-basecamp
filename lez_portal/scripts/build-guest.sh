#!/usr/bin/env bash
# Build the portal guest and package it as target/guest/portal.bin, WITHOUT Docker.
#
# Needs the risc0 toolchain: rzup install cargo-risczero 3.0.5 && rzup install rust
# (and `rzup install cpp` if a C dependency of the guest needs the riscv gcc).
#
# The Docker build of lez-programs (scripts/build-guests.sh there) gives a reproducible
# ImageID; this one gives a working binary for a local chain, but its ImageID may differ.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
target_triple="riscv32im-risc0-zkvm-elf"
us="$(printf '\037')"

export CARGO_ENCODED_RUSTFLAGS="-C${us}passes=lower-atomic${us}-C${us}link-arg=-Ttext=0x00200800${us}-C${us}link-arg=--fatal-warnings${us}-C${us}panic=abort${us}--cfg${us}getrandom_backend=\"custom\""
export RISC0_FEATURE_bigint2=""
if [[ -x "${HOME}/.risc0/cpp/bin/riscv32-unknown-elf-gcc" ]]; then
  export CC_riscv32im_risc0_zkvm_elf="${HOME}/.risc0/cpp/bin/riscv32-unknown-elf-gcc"
  export CFLAGS_riscv32im_risc0_zkvm_elf="-march=rv32im -nostdlib"
fi
export CARGO_TARGET_DIR="${here}/target/guest-build"

cargo +risc0 build --release --target "${target_triple}" \
  --manifest-path "${here}/methods/guest/Cargo.toml"

unset CARGO_ENCODED_RUSTFLAGS
cargo build --manifest-path "${here}/tools/risc0-packager/Cargo.toml" \
  --target-dir "${here}/target/packager"

mkdir -p "${here}/target/guest"
"${here}/target/packager/debug/risc0-packager" \
  "${CARGO_TARGET_DIR}/${target_triple}/release/portal" \
  "${here}/target/guest/portal.bin"
echo "built ${here}/target/guest/portal.bin"
