use crate::vga_buffer::{self, Color};
use core::sync::atomic::{AtomicU64, Ordering};

static TIMER_COUNT: AtomicU64 = AtomicU64::new(0);

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

pub fn rainbow_text(row: usize, col: usize, text: &str, frame: u64) {
    let colors = [
        Color::Red,
        Color::LightRed,
        Color::Yellow,
        Color::Green,
        Color::LightGreen,
        Color::Cyan,
        Color::LightBlue,
        Color::Blue,
        Color::Magenta,
        Color::Pink,
    ];
    
    for (i, byte) in text.bytes().enumerate() {
        if byte == b' ' {
            continue;
        }
        
        let color_index = ((i as u64 + frame) % colors.len() as u64) as usize;
        vga_buffer::write_at(row, col + i, byte, colors[color_index], Color::Black);
    }
}

pub fn draw_box(start_row: usize, start_col: usize, width: usize, height: usize, color: Color) {
    // Draw top and bottom borders
    for col in start_col..start_col + width {
        vga_buffer::write_at(start_row, col, b'-', color, Color::Black);
        vga_buffer::write_at(start_row + height - 1, col, b'-', color, Color::Black);
    }
    
    // Draw left and right borders
    for row in start_row..start_row + height {
        vga_buffer::write_at(row, start_col, b'|', color, Color::Black);
        vga_buffer::write_at(row, start_col + width - 1, b'|', color, Color::Black);
    }
    
    // Draw corners
    vga_buffer::write_at(start_row, start_col, b'+', color, Color::Black);
    vga_buffer::write_at(start_row, start_col + width - 1, b'+', color, Color::Black);
    vga_buffer::write_at(start_row + height - 1, start_col, b'+', color, Color::Black);
    vga_buffer::write_at(start_row + height - 1, start_col + width - 1, b'+', color, Color::Black);
}

pub fn display_fancy_welcome() {
    vga_buffer::clear_screen();

    // Draw the CottonOS logo
    draw_cotton_logo(2, 10);
    
    // Let's make sure text is visible by using known good positions and simplified code
    // Basic welcome message
    vga_buffer::write_str_at(12, 20, "Welcome to CottonOS!", Color::White, Color::Black);
    vga_buffer::write_str_at(13, 20, "A Rust-based Operating System", Color::Yellow, Color::Black);
    
    // Explicitly add the GitHub URL - keeping it simple
    vga_buffer::write_str_at(15, 10, "IMPORTANT: This OS is under active development!", Color::LightRed, Color::Black);
    vga_buffer::write_str_at(16, 10, "Contribute at: github.com/aryansrao/CottonOs", Color::LightGreen, Color::Black);
    

    // Add some delay to ensure visibility
    sleep(1000);
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
