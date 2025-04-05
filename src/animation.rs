use crate::vga_buffer::{self, Color};
use core::sync::atomic::{AtomicU64, AtomicBool, Ordering};

static TIMER_COUNT: AtomicU64 = AtomicU64::new(0);
static WELCOME_SHOWN: AtomicBool = AtomicBool::new(false);
// A global flag to directly control welcome screen display
static mut BYPASS_WELCOME: bool = false;
// Counter for welcome screen attempts
static mut WELCOME_ATTEMPT_COUNT: u8 = 0;

pub fn increment_timer() {
    TIMER_COUNT.fetch_add(1, Ordering::SeqCst);
}

pub fn get_timer() -> u64 {
    TIMER_COUNT.load(Ordering::SeqCst)
}

pub fn sleep(ms: u64) {
    let start = get_timer();
    while get_timer() - start < ms {}
}

// Add a function to draw a cotton logo
pub fn draw_cotton_logo(start_row: usize, start_col: usize) {
    // ASCII art for CottonOS - fixed to ensure the "n" is visible
    let logo = [
        "   _____      _   _                ____   _____ ",
        "  / ____|    | | | |              / __ \\ / ____|",
        " | |     ___ | |_| |_ ___  _ __  | |  | | (___  ",
        " | |    / _ \\| __| __/ _ \\| '_ \\ | |  | |\\___ \\ ",
        " | |___| (_) | |_| || (_) | | | || |__| |____) |",
        "  \\_____\\___/ \\__|\\__\\___/|_| |_| \\____/|_____/ ",
    ];

    // Draw each line of the logo with extra spacing
    for (i, line) in logo.iter().enumerate() {
        for (j, ch) in line.bytes().enumerate() {
            let color = match i {
                0 | 5 => Color::LightCyan,
                1 | 4 => Color::Cyan,
                _ => Color::Blue,
            };
            
            if ch != b' ' {
                vga_buffer::write_at(
                    start_row + i,
                    start_col + j,
                    ch,
                    color,
                    Color::Black
                );
            }
        }
    }
}

// Add a debug function to print the current state of the welcome flag
pub fn debug_welcome_flag() -> bool {
    WELCOME_SHOWN.load(Ordering::SeqCst)
}

// Make the disable_welcome_screen function even more forceful
pub fn disable_welcome_screen() {
    // First set the atomic flag with sequential consistency
    WELCOME_SHOWN.store(true, Ordering::SeqCst);
    
    // Double-check that it got set, if not try other orderings
    if !WELCOME_SHOWN.load(Ordering::SeqCst) {
        // Try again with different memory orderings
        WELCOME_SHOWN.store(true, Ordering::Release);
        WELCOME_SHOWN.store(true, Ordering::Relaxed);
    }
    
    // Also explicitly set our bypass flag
    unsafe {
        BYPASS_WELCOME = true;
        // Reset welcome attempt count for good measure
        WELCOME_ATTEMPT_COUNT = 0;
    }
    
    // Add a small delay to ensure the change propagates
    for _ in 0..1000 {
        core::hint::spin_loop();
    }
}

// Add a function to reset the welcome screen flag
pub fn reset_welcome_screen() {
    WELCOME_SHOWN.store(false, Ordering::SeqCst);
    // Reset our bypass flag as well
    unsafe {
        BYPASS_WELCOME = false;
    }
}

// Simple welcome screen with ASCII art
pub fn display_welcome_screen() {
    // Extra initial bypass check
    unsafe {
        if BYPASS_WELCOME {
            // If bypass is set, always skip welcome
            return;
        }
    }

    // Check if we already showed the welcome screen this boot
    if WELCOME_SHOWN.load(Ordering::SeqCst) {
        // Skip showing welcome screen to avoid disrupting command flow
        return;
    }
    
    // Make sure we don't get stuck in a loop
    unsafe {
        WELCOME_ATTEMPT_COUNT += 1;
        if WELCOME_ATTEMPT_COUNT > 1 {
            // If we've already tried to show the welcome screen more than once
            // since boot, force the flag to true and return
            WELCOME_SHOWN.store(true, Ordering::SeqCst);
            return;
        }
    }
    
    vga_buffer::clear_screen();

    // Draw the CottonOS logo
    draw_cotton_logo(5, 10);
    
    // Display a simple welcome message
    vga_buffer::write_str_at(12, 20, "Welcome to CottonOS!", Color::White, Color::Black);
    vga_buffer::write_str_at(13, 20, "A Rust-based Operating System", Color::Yellow, Color::Black);
    
    // GitHub URL
    vga_buffer::write_str_at(16, 10, "Contribute at: github.com/aryansrao/CottonOs", Color::LightGreen, Color::Black);
    
    // Shell instruction
    vga_buffer::write_str_at(19, 20, "Press SPACE to start the shell...", Color::White, Color::Black);
    
    // Wait for spacebar specifically
    loop {
        let key = crate::keyboard::wait_for_keypress();
        if key == crate::keyboard::SCAN_SPACE {
            break;
        }
    }
    
    // Make sure this flag is set
    WELCOME_SHOWN.store(true, Ordering::SeqCst);
}

// Force-display welcome screen regardless of flags
pub fn force_welcome_screen() {
    // Reset flags first
    WELCOME_SHOWN.store(false, Ordering::SeqCst);
    
    // Reset bypass flag 
    unsafe {
        BYPASS_WELCOME = false;
        // Reset the attempt counter
        WELCOME_ATTEMPT_COUNT = 0;
    }
    
    // Clear screen
    vga_buffer::clear_screen();

    // Draw the CottonOS logo and rest of welcome screen
    draw_cotton_logo(5, 10);
    
    vga_buffer::write_str_at(12, 20, "Welcome to CottonOS!", Color::White, Color::Black);
    vga_buffer::write_str_at(13, 20, "A Rust-based Operating System", Color::Yellow, Color::Black);
    vga_buffer::write_str_at(16, 10, "Contribute at: github.com/aryansrao/CottonOs", Color::LightGreen, Color::Black);
    vga_buffer::write_str_at(19, 20, "Press SPACE to start the shell...", Color::White, Color::Black);
    
    // Wait for spacebar
    loop {
        let key = crate::keyboard::wait_for_keypress();
        if key == crate::keyboard::SCAN_SPACE {
            break;
        }
    }
    
    // Set the flag to true after showing
    WELCOME_SHOWN.store(true, Ordering::SeqCst);
}

// Simple shell prompt
pub fn display_shell_prompt(line: usize) {
    vga_buffer::write_str_at(line, 0, "cottonos> ", Color::Green, Color::Black);
}

// Function to wait for user input
pub fn wait_for_keypress() -> u8 {
    crate::keyboard::wait_for_keypress()
}
