#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(cotton_os::test_runner)]
#![reexport_test_harness_main = "test_main"]

use core::panic::PanicInfo;
use cotton_os::{println, animation};
use cotton_os::vga_buffer::{Color, clear_screen, write_str_at, write_at};
use cotton_os::keyboard::{wait_for_keypress, scan_code_to_char, SCAN_ENTER, SCAN_BACKSPACE};

// Only keep the constants we actually use
const BUFFER_HEIGHT: usize = 25;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Initialize keyboard handling
    cotton_os::keyboard::init();
    
    // Make sure the welcome screen flag is reset at boot
    animation::reset_welcome_screen();
    
    // Display welcome screen and wait for spacebar
    animation::display_welcome_screen();
    
    #[cfg(test)]
    test_main();

    // Clear screen for shell
    clear_screen();
    
    // Start shell - this should never return
    run_shell();
}

// Simple shell implementation
fn run_shell() -> ! {
    let mut line = 0;
    let mut cmd_position = 0;
    let mut current_cmd = [0u8; 80];
    
    // Display initial prompt
    animation::display_shell_prompt(line);
    
    loop {
        let key = wait_for_keypress();
        
        if key == SCAN_ENTER {
            // Process the command (in a real shell, you'd parse and execute it)
            process_command(&current_cmd[0..cmd_position]);
            
            // Move to next line and reset command buffer
            line = (line + 1) % BUFFER_HEIGHT;
            if line == 0 {
                // Screen is full, clear and start over
                clear_screen();
            }
            
            // Display new prompt
            animation::display_shell_prompt(line);
            cmd_position = 0;
            current_cmd = [0u8; 80];
        } 
        else if key == SCAN_BACKSPACE && cmd_position > 0 {
            // Handle backspace - remove the last character
            cmd_position -= 1;
            
            // Clear the character on screen
            write_at(line, 9 + cmd_position, b' ', Color::White, Color::Black);
            
            // Update the cursor position - we don't actually move a cursor,
            // but we need to ensure the next character replaces this position
        }
        else if let Some(ch) = scan_code_to_char(key) {
            // Add character to command buffer if there's space
            if cmd_position < 70 { // Leave some margin
                let ch_byte = ch as u8;
                write_at(line, 9 + cmd_position, ch_byte, Color::White, Color::Black);
                current_cmd[cmd_position] = ch_byte;
                cmd_position += 1;
            }
        }
        
        // Add a small delay to avoid CPU spinning too fast
        for _ in 0..1000 {
            core::hint::spin_loop();
        }
    }
}

// Process shell commands
fn process_command(cmd: &[u8]) {
    // Check for built-in commands
    if starts_with(cmd, b"help") {
        write_str_at(get_current_line() + 1, 0, "Available commands: help, clear, about, sysinfo, devices, exit", Color::Yellow, Color::Black);
    } 
    else if starts_with(cmd, b"clear") {
        clear_screen();
    }
    else if starts_with(cmd, b"about") {
        write_str_at(get_current_line() + 1, 0, "CottonOS - A simple Rust-based OS", Color::LightCyan, Color::Black);
        write_str_at(get_current_line() + 2, 0, "Created by: Aryan Rao", Color::LightCyan, Color::Black);
        write_str_at(get_current_line() + 3, 0, "GitHub: github.com/aryansrao/CottonOs", Color::LightGreen, Color::Black);
    }
    else if starts_with(cmd, b"exit") {
        // Use the force_welcome_screen function instead
        animation::force_welcome_screen();
        
        // Clear the screen after welcome screen
        clear_screen();
        
        // Return to shell
        return;
    }
    else if starts_with(cmd, b"devices") {
        // Use our enhanced device detection system
        detect_and_display_devices();
    }
    else if starts_with(cmd, b"sysinfo") {
        // First ensure welcome screen won't show
        animation::disable_welcome_screen();

        // Clear screen 
        clear_screen();
        
        // Display a fixed system info page
        write_str_at(0, 0, "==== CottonOS System Information ====", Color::White, Color::Blue);
        write_str_at(2, 0, "System: CottonOS v0.1.0", Color::White, Color::Black);
        write_str_at(3, 0, "Kernel: Rust Bare Metal", Color::White, Color::Black);
        write_str_at(4, 0, "Architecture: x86_64", Color::White, Color::Black);
        
        write_str_at(6, 0, "--- CPU Information ---", Color::Yellow, Color::Black);
        write_str_at(7, 0, "Model: QEMU Virtual CPU", Color::White, Color::Black);
        write_str_at(8, 0, "Features: SSE, SSE2", Color::White, Color::Black);
        
        write_str_at(10, 0, "--- Memory Information ---", Color::Yellow, Color::Black);
        write_str_at(11, 0, "Total RAM: 640KB (Fixed allocation)", Color::White, Color::Black);
        write_str_at(12, 0, "Used: <unknown>", Color::White, Color::Black);
        
        write_str_at(14, 0, "--- Hardware Information ---", Color::Yellow, Color::Black);
        write_str_at(15, 0, "Video: VGA Text Mode (80x25)", Color::White, Color::Black);
        write_str_at(16, 0, "Input: PS/2 Keyboard", Color::White, Color::Black);
        
        write_str_at(18, 0, "--- OS Statistics ---", Color::Yellow, Color::Black);
        write_str_at(19, 0, "Uptime: N/A", Color::White, Color::Black);
        write_str_at(20, 0, "Build: Rust nightly", Color::White, Color::Black);

        // Show a prompt to return
        write_str_at(23, 0, "Press Enter to return to shell...", Color::White, Color::Black);
        
        // Disable welcome screen again for good measure
        animation::disable_welcome_screen();

        // Wait for Enter key
        loop {
            let k = wait_for_keypress();
            if k == SCAN_ENTER {
                // Make sure welcome screen isn't shown
                animation::disable_welcome_screen();
                break;
            }
        }
        
        // Clear screen and return to shell
        clear_screen();
    }
    else if !cmd.is_empty() {
        // Command not recognized
        write_str_at(get_current_line() + 1, 0, "Command not found. Type 'help' for available commands.", Color::Red, Color::Black);
    }
}

// Function to detect and display all connected devices - completely rewritten for stability
fn detect_and_display_devices() {
    // First, disable welcome screen
    animation::disable_welcome_screen();
    
    // Clear screen
    clear_screen();
    
    // Display the header with 100% static strings
    write_str_at(0, 0, "==== CottonOS Device Detection ====", Color::White, Color::Blue);
    
    // Display a simple, fixed device list with no dynamic data
    write_str_at(2, 0, "--- Input Devices ---", Color::Yellow, Color::Black);
    write_str_at(3, 2, "Device: PS/2 Keyboard", Color::White, Color::Black);
    write_str_at(4, 4, "Status: Connected", Color::LightGreen, Color::Black);
    write_str_at(5, 4, "Connected since boot", Color::White, Color::Black);
    write_str_at(6, 4, "Driver: PS/2 Controller", Color::White, Color::Black);
    
    write_str_at(8, 0, "--- Output Devices ---", Color::Yellow, Color::Black);
    write_str_at(9, 2, "Device: VGA Text Mode Display", Color::White, Color::Black);
    write_str_at(10, 4, "Status: Active", Color::LightGreen, Color::Black);
    write_str_at(11, 4, "Resolution: 80x25 characters", Color::White, Color::Black);
    
    write_str_at(13, 0, "--- Storage Devices ---", Color::Yellow, Color::Black);
    write_str_at(14, 2, "Device: Boot Medium", Color::White, Color::Black);
    write_str_at(15, 4, "Status: Read-only", Color::LightCyan, Color::Black);
    write_str_at(16, 4, "Size: Unknown", Color::White, Color::Black);
    
    // Show prompt to return
    write_str_at(BUFFER_HEIGHT - 2, 0, "Press Enter to return to shell...", Color::White, Color::Black);
    
    // Wait for Enter key, with safety check
    loop {
        animation::disable_welcome_screen();
        if wait_for_keypress() == SCAN_ENTER {
            break;
        }
    }
    
    // Clear screen and return
    animation::disable_welcome_screen();
    clear_screen();
}

// Helper function to check if a byte slice starts with another byte slice
fn starts_with(input: &[u8], prefix: &[u8]) -> bool {
    input.len() >= prefix.len() && &input[0..prefix.len()] == prefix
}

// Helper function to get current line (last line with content)
fn get_current_line() -> usize {
    // In a simple implementation, we don't track this properly
    // This would need to be enhanced in a real shell
    0 // Placeholder
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
