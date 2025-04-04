#!/bin/bash

# Exit on error
set -e

# Check for Rosetta installation on M-series Macs
if [[ $(uname -m) == "arm64" ]]; then
    echo "Running on Apple Silicon Mac."
    echo "Note: Cross-compilation from ARM to x86_64 might have issues."
    echo "For best results, use the standard ARM toolchain."
fi

# Install required tools
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly
rustup component add llvm-tools-preview --toolchain nightly

# Clean previous build artifacts
echo "Cleaning previous build artifacts..."
rm -rf target

# Install bootimage if not already installed
if ! cargo install --list | grep -q "bootimage"; then
    cargo install bootimage
fi

echo "Building CottonOS..."

# Build with standard ARM toolchain - it should work with proper target spec
cargo +nightly build -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem

echo "Creating bootimage..."
cargo bootimage

echo "Build successful! Run with:"
echo "qemu-system-x86_64 -drive format=raw,file=target/x86_64-cotton_os/debug/bootimage-cotton_os.bin"
