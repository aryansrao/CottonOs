.PHONY: build run test clean

build:
	rustup toolchain install nightly
	rustup component add rust-src --toolchain nightly
	cargo +nightly build -Z build-std=core,compiler_builtins,alloc -Z build-std-features=compiler-builtins-mem

run: build
	cargo +nightly run

test:
	cargo +nightly test

clean:
	cargo clean
	rm -rf target

# For Apple Silicon Macs
run-qemu: build
	qemu-system-x86_64 -drive format=raw,file=target/x86_64-cotton_os/debug/bootimage-cotton_os.bin -m 128M
