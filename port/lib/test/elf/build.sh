#!/bin/sh
# Rebuild hello.elf from hello.rs and hello.ld, with the pinned toolchain's
# rustc and rust-lld.
set -e
cd "$(dirname "$0")"
rustc --edition 2024 --target aarch64-unknown-none -C panic=abort \
	-C opt-level=s -C relocation-model=static -C strip=symbols \
	-C link-arg=-Thello.ld -C link-arg=-zmax-page-size=4096 \
	-o hello.elf hello.rs
