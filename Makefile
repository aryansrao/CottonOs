.PHONY: all build clean run test bootimage

# Default target
all: build

# Build the kernel
build:
	rustup toolchain install nightly
	rustup component add rust-src --toolchain nightly
	cargo +nightly build 

# Create bootimage
bootimage: build
	cargo install bootimage
	cargo bootimage

# Clean build artifacts
clean:
	cargo clean

# Run in QEMU
run: bootimage
	qemu-system-x86_64 -drive format=raw,file=target/x86_64-cotton_os/debug/bootimage-cotton_os.bin

# Run with serial output
run-serial: bootimage
	qemu-system-x86_64 -drive format=raw,file=target/x86_64-cotton_os/debug/bootimage-cotton_os.bin -serial stdio

# Test in QEMU
test:
	cargo test
