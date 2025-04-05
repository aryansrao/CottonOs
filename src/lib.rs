#![no_std]
#![cfg_attr(test, no_main)]
#![feature(custom_test_frameworks)]
#![feature(alloc_error_handler)]
#![test_runner(crate::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

use core::panic::PanicInfo;
use core::alloc::{GlobalAlloc, Layout};

pub mod vga_buffer;
pub mod serial;
pub mod animation;
pub mod keyboard;
pub mod devices;

// Basic allocator for use by the alloc crate
struct DummyAllocator;

unsafe impl GlobalAlloc for DummyAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Use fixed size constants instead of accessing via .len()
        const HEAP_SIZE: usize = 8192;
        static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
        static mut NEXT_FREE: usize = 0;
        
        let start = NEXT_FREE;
        let align = layout.align();
        let size = layout.size();
        
        // Make sure we have proper alignment (very important!)
        let align_mask = align - 1;
        let aligned_start = (start + align_mask) & !align_mask;
        let end = aligned_start + size;
        
        // Check if we have enough space
        if end > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        
        // Update next free position
        NEXT_FREE = end;
        
        // Get pointer to HEAP without using any references
        let heap_ptr = core::ptr::addr_of_mut!(HEAP) as *mut u8;
        
        // Return aligned pointer
        if heap_ptr.is_null() {
            core::ptr::null_mut()
        } else {
            heap_ptr.add(aligned_start)
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // This allocator doesn't support deallocation
    }
}

#[global_allocator]
static ALLOCATOR: DummyAllocator = DummyAllocator;

#[alloc_error_handler]
fn alloc_error_handler(_layout: Layout) -> ! {
    loop {} // Just hang on allocation error
}

pub trait Testable {
    fn run(&self) -> ();
}

impl<T> Testable for T
where
    T: Fn(),
{
    fn run(&self) {
        serial_print!("{}...\t", core::any::type_name::<T>());
        self();
        serial_println!("[ok]");
    }
}

pub fn test_runner(tests: &[&dyn Testable]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test.run();
    }
    exit_qemu(QemuExitCode::Success);
}

pub fn test_panic_handler(info: &PanicInfo) -> ! {
    serial_println!("[failed]\n");
    serial_println!("Error: {}\n", info);
    exit_qemu(QemuExitCode::Failed);
    loop {}
}

// Debug helper function to help check system status
pub fn debug_status() -> bool {
    animation::debug_welcome_flag()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}

#[cfg(test)]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    test_main();
    loop {}
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    test_panic_handler(info)
}
