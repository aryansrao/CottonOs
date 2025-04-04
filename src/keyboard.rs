use core::sync::atomic::{AtomicU8, Ordering};
use x86_64::instructions::port::Port;

// Keyboard port for reading scan codes
const KEYBOARD_PORT: u16 = 0x60;

// Last keyboard scan code received
static LAST_SCAN_CODE: AtomicU8 = AtomicU8::new(0);
static KEY_PRESSED: AtomicU8 = AtomicU8::new(0);

// Common scan codes for basic keyboard input
pub const SCAN_ESC: u8 = 0x01;
pub const SCAN_ENTER: u8 = 0x1C;
pub const SCAN_SPACE: u8 = 0x39;
pub const SCAN_BACKSPACE: u8 = 0x0E; // Add backspace scan code
pub const SCAN_UP: u8 = 0x48;
pub const SCAN_DOWN: u8 = 0x50;
pub const SCAN_LEFT: u8 = 0x4B;
pub const SCAN_RIGHT: u8 = 0x4D;

// Initialize the keyboard handler
pub fn init() {
    // Nothing to initialize for now, but we'll leave this here for future expansion
}

// Check if a key is available in the keyboard buffer
pub fn is_key_available() -> bool {
    // For QEMU, we need a more reliable method to check keyboard status
    unsafe {
        // Read the keyboard controller status register
        let mut status_port = Port::new(0x64);
        let status: u8 = status_port.read();
        
        // Check if output buffer is full (bit 0)
        if status & 1 == 0 {
            return false;
        }
        
        // Check if the data is from keyboard (not mouse)
        if status & 0x20 != 0 {
            return false;
        }
        
        true
    }
}

// Simple function to poll for a keypress and return the scan code
pub fn wait_for_keypress() -> u8 {
    // For better reliability, let's implement a primitive keyboard polling loop
    loop {
        // Direct check for keyboard data
        unsafe {
            // First check keyboard controller status
            let mut status_port = Port::new(0x64);
            let status: u8 = status_port.read();
            
            // If output buffer has data and it's from keyboard
            if status & 1 != 0 && status & 0x20 == 0 {
                let mut data_port = Port::new(0x60);
                let scan_code: u8 = data_port.read();
                
                // Only react to key press (not key release)
                if scan_code < 0x80 {
                    LAST_SCAN_CODE.store(scan_code, Ordering::SeqCst);
                    KEY_PRESSED.store(1, Ordering::SeqCst);
                    return scan_code;
                }
            }
        }
        
        // Yield CPU time to avoid 100% CPU usage
        for _ in 0..10_000 {
            core::hint::spin_loop();
        }
    }
}

// Check if a key was pressed and store the scan code
pub fn check_for_keypress() -> bool {
    if is_key_available() {
        let mut port = Port::new(KEYBOARD_PORT);
        let scan_code: u8 = unsafe { port.read() };
        
        // Store the scan code for reference
        LAST_SCAN_CODE.store(scan_code, Ordering::SeqCst);
        
        // Check if this is a key press (not a key release)
        if scan_code < 0x80 {
            KEY_PRESSED.store(1, Ordering::SeqCst);
            return true;
        }
    }
    false
}

// Get the last pressed key's scan code
pub fn get_last_scan_code() -> u8 {
    LAST_SCAN_CODE.load(Ordering::SeqCst)
}

// Simple conversion of scan code to ASCII (very limited!)
pub fn scan_code_to_char(scan_code: u8) -> Option<char> {
    match scan_code {
        0x1E => Some('a'),
        0x30 => Some('b'),
        0x2E => Some('c'),
        0x20 => Some('d'),
        0x12 => Some('e'),
        0x21 => Some('f'),
        0x22 => Some('g'),
        0x23 => Some('h'),
        0x17 => Some('i'),
        0x24 => Some('j'),
        0x25 => Some('k'),
        0x26 => Some('l'),
        0x32 => Some('m'),
        0x31 => Some('n'),
        0x18 => Some('o'),
        0x19 => Some('p'),
        0x10 => Some('q'),
        0x13 => Some('r'),
        0x1F => Some('s'),
        0x14 => Some('t'),
        0x16 => Some('u'),
        0x2F => Some('v'),
        0x11 => Some('w'),
        0x2D => Some('x'),
        0x15 => Some('y'),
        0x2C => Some('z'),
        0x02..=0x0A => Some(((scan_code - 0x02) + b'1') as char), // 1-9
        0x0B => Some('0'),               // Fixed overlapping pattern
        SCAN_SPACE => Some(' '),
        SCAN_ENTER => Some('\n'),
        _ => None,
    }
}
