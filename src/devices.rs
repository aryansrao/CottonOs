//! Device detection module for CottonOS
//!
//! This module provides functionality to detect and manage hardware devices
//! connected to the system.

use alloc::string::String;
use x86_64::instructions::port::Port;

// Simplified device registry without mutex locking issues
static mut DEVICE_LIST: Option<DeviceCollection> = None;

/// Main function to detect all hardware devices
pub fn detect_all_devices() -> DeviceCollection {
    // Check if we already detected devices
    unsafe {
        if let Some(ref devices) = DEVICE_LIST {
            return devices.clone();
        }
    }
    
    // Create new device collection
    let mut devices = DeviceCollection::new();
    
    // Detect PS/2 devices
    detect_ps2_devices(&mut devices);
    
    // Detect display adapters
    detect_display_devices(&mut devices);
    
    // Detect storage devices
    detect_storage_devices(&mut devices);
    
    // Store and return the collection
    unsafe {
        DEVICE_LIST = Some(devices.clone());
    }
    
    devices
}

/// Detects PS/2 devices like keyboard and mouse
fn detect_ps2_devices(devices: &mut DeviceCollection) {
    // Check PS/2 controller status
    let mut status_port = Port::<u8>::new(0x64);
    let status = unsafe { status_port.read() };
    
    // Check if PS/2 controller exists
    if (status & 0x04) == 0 {
        // PS/2 controller is present
        
        // Check for keyboard
        let mut data_port = Port::<u8>::new(0x60);
        
        // Send keyboard self-test command
        let mut cmd_port = Port::<u8>::new(0x64);
        unsafe {
            // Wait for controller to be ready
            while (status_port.read() & 0x02) != 0 {}
            
            // Send keyboard identification command
            cmd_port.write(0xAE); // Enable first PS/2 port
            
            // Wait for response
            for _ in 0..1000 {
                if (status_port.read() & 0x01) != 0 {
                    // Data available - ignore response value
                    let _ = data_port.read();
                    
                    // PS/2 keyboard detected
                    devices.add_input_device(
                        "PS/2 Keyboard",
                        "Connected",
                        true,
                        Some("Basic PS/2 driver"),
                        Some("Supports standard keystrokes")
                    );
                    break;
                }
                
                // Small delay
                for _ in 0..1000 {
                    core::hint::spin_loop();
                }
            }
        }
        
        // For a real system, we would also check for PS/2 mouse, but we'll
        // simplify here and just report a mouse is present for demo purposes
        devices.add_input_device(
            "PS/2 Mouse",
            "Connected",
            true,
            Some("Basic PS/2 mouse driver"),
            Some("Supports relative movement")
        );
    }
}

/// Detects video display adapters
fn detect_display_devices(devices: &mut DeviceCollection) {
    // For a VGA text mode display, we can check the standard I/O ports
    devices.add_output_device(
        "VGA Text Mode Display",
        "Active",
        true,
        Some("VGA text mode driver"),
        Some("Resolution: 80x25 characters")
    );
}

/// Detects storage devices like disks
fn detect_storage_devices(devices: &mut DeviceCollection) {
    // In a real OS, we'd scan PCI bus, SATA controllers, etc.
    // For our simple OS, we'll just assume a boot device exists
    
    devices.add_storage_device(
        "Boot Medium (QEMU virtual)",
        "Read-only",
        true,
        Some("Basic storage driver"),
        Some("Unknown")
    );
}

/// Type of device (input, output, storage, etc)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Input,
    Output,
    Storage,
}

/// Represents a single hardware device
#[derive(Clone, Debug)]
pub struct Device {
    pub device_type: DeviceType,
    name: String,
    status: String,
    active: bool,
    driver: Option<String>,
    info: Option<String>,
    size: Option<String>,
}

impl Device {
    /// Create a new device
    fn new(
        device_type: DeviceType,
        name: &str,
        status: &str,
        active: bool,
        driver: Option<&str>,
        info: Option<&str>,
        size: Option<&str>,
    ) -> Self {
        Device {
            device_type,
            name: String::from(name),
            status: String::from(status),
            active,
            driver: driver.map(String::from),
            info: info.map(String::from),
            size: size.map(String::from),
        }
    }
    
    /// Get device name
    pub fn name(&self) -> &str {
        &self.name
    }
    
    /// Get device status
    pub fn status(&self) -> &str {
        &self.status
    }
    
    /// Check if device is active
    pub fn is_active(&self) -> bool {
        self.active
    }
    
    /// Get device driver if available
    pub fn driver(&self) -> Option<&str> {
        self.driver.as_deref()
    }
    
    /// Get additional device information if available
    pub fn additional_info(&self) -> Option<&str> {
        self.info.as_deref()
    }
    
    /// Get device size if applicable (for storage devices)
    pub fn size(&self) -> Option<&str> {
        self.size.as_deref()
    }
}

/// Collection of devices organized by type for display
#[derive(Clone)]
pub struct DeviceCollection {
    input: Vec<Device>,
    output: Vec<Device>,
    storage: Vec<Device>,
}

impl DeviceCollection {
    /// Create a new empty device collection
    fn new() -> Self {
        DeviceCollection {
            input: Vec::new(),
            output: Vec::new(),
            storage: Vec::new(),
        }
    }
    
    /// Add an input device
    fn add_input_device(
        &mut self,
        name: &str,
        status: &str,
        active: bool,
        driver: Option<&str>,
        info: Option<&str>,
    ) {
        self.input.push(Device::new(
            DeviceType::Input, 
            name, 
            status, 
            active,
            driver,
            info,
            None
        ));
    }
    
    /// Add an output device
    fn add_output_device(
        &mut self,
        name: &str,
        status: &str,
        active: bool,
        driver: Option<&str>,
        info: Option<&str>,
    ) {
        self.output.push(Device::new(
            DeviceType::Output, 
            name, 
            status, 
            active,
            driver,
            info,
            None
        ));
    }
    
    /// Add a storage device
    fn add_storage_device(
        &mut self,
        name: &str,
        status: &str,
        active: bool,
        driver: Option<&str>,
        size: Option<&str>,
    ) {
        self.storage.push(Device::new(
            DeviceType::Storage, 
            name, 
            status, 
            active,
            driver,
            Some("Storage device"),
            size
        ));
    }
    
    /// Get all input devices
    pub fn input_devices(&self) -> &[Device] {
        &self.input
    }
    
    /// Get all output devices
    pub fn output_devices(&self) -> &[Device] {
        &self.output
    }
    
    /// Get all storage devices
    pub fn storage_devices(&self) -> &[Device] {
        &self.storage
    }
}

/// Import the Vec type from the alloc crate
use alloc::vec::Vec;
