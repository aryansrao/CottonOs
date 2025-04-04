#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(cotton_os::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;
use cotton_os::println;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    println!("Welcome to CottonOS - A Rust-based mobile operating system");
    
    #[cfg(test)]
    test_main();

    loop {}
}

/// This function is called on panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    loop {}
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    cotton_os::test_panic_handler(info)
}
