# CottonOS - A Rust-based Mobile Operating System

## Project Overview
CottonOS is an experimental operating system for mobile devices written in Rust. The project aims to create a secure, efficient, and modern OS specifically optimized for mobile hardware.

## Getting Started

### Prerequisites
- Rust (nightly): Install using rustup
- QEMU: For testing the OS in a virtual environment
- Bootimage: `cargo install bootimage`
- Additional dependencies: `rustup component add rust-src llvm-tools-preview`

### Building the OS
```bash
cargo build
```

### Running in QEMU
```bash
cargo run
```

## Project Structure
- `src/main.rs`: Entry point of the kernel
- `src/lib.rs`: Core OS library functions
- `src/vga_buffer.rs`: Interface for text output
- `src/serial.rs`: Serial port interface for debugging

## Roadmap

### Phase 1: Basic Kernel
- [x] Bootable kernel
- [x] VGA text output
- [x] Serial debugging
- [ ] Interrupts and exceptions
- [ ] Memory management (paging)

### Phase 2: Hardware Interface
- [ ] Device drivers
- [ ] Graphics interface
- [ ] Touch screen support
- [ ] Audio support
- [ ] Power management

### Phase 3: OS Fundamentals
- [ ] Process management
- [ ] Scheduling
- [ ] File system
- [ ] Networking stack

### Phase 4: Mobile-specific Features
- [ ] Mobile UI system
- [ ] Application framework
- [ ] Security features
- [ ] Battery optimization

## Contributing
Contributions are welcome! Please feel free to submit a Pull Request.

## License
This project is open source, licensed under the MIT License.
