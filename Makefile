.PHONY: build run test clean bootimage run-qemu

build:
	rustup toolchain install nightly
	rustup component add rust-src --toolchain nightly
	cargo +nightly build -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem

bootimage: build
	cargo bootimage

run: build
	cargo +nightly run

test:
	cargo +nightly test

clean:
	cargo clean
	rm -rf target

# For Apple Silicon Macs
run-qemu: bootimage
	qemu-system-x86_64 -drive format=raw,file=target/x86_64-cotton_os/debug/bootimage-cotton_os.bin -m 128M
