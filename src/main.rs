#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(cotton_os::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;
use cotton_os::{println, animation};
use cotton_os::vga_buffer::Color;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Display welcome message
    animation::display_fancy_welcome();
    
    #[cfg(test)]
    test_main();

    // Define colors for animation
    let colors = [
        Color::Red,
        Color::Yellow, 
        Color::Green,
        Color::Cyan,
        Color::Blue,
        Color::Magenta,
        Color::White,
    ];

    // After animation, always make sure GitHub URL is visible
    // Put it at a fixed position that works reliably
    let github_msg = "Help us develop: github.com/aryansrao/CottonOs";
    cotton_os::vga_buffer::write_str_at(
        BUFFER_HEIGHT - 1,  // Bottom line
        5,                  // Near the left edge
        github_msg,
        Color::LightGreen,
        Color::Black
    );

    // After animation, enter main loop
    loop {
        // Simple animation in the main loop
        animation::increment_timer();
        
        let timer = animation::get_timer();
        
        // Update status message periodically
        if timer % 2000 == 0 {
            let color_index = ((timer / 2000) % colors.len() as u64) as usize;
            
            // Simple running indicator at the bottom right
            cotton_os::vga_buffer::write_str_at(
                BUFFER_HEIGHT - 1,           // Bottom line
                BUFFER_WIDTH - 20,           // Right side
                "CottonOS Running",
                colors[color_index],
                Color::Black
            );
        }
        
        // Add a small delay
        for _ in 0..100 {
            core::hint::spin_loop();
        }
    }
}

// Constants for screen dimensions
const BUFFER_HEIGHT: usize = 25;
const BUFFER_WIDTH: usize = 80;

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
