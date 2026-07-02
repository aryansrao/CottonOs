//! Kernel Heap Allocator
//!
//! Provides dynamic memory allocation for the kernel using linked_list_allocator.

use linked_list_allocator::LockedHeap;
use crate::mm::{PAGE_SIZE, physical};

/// Heap start address (identity mapped by the boot page tables)
pub const HEAP_START: u64 = 0x0000_0000_0200_0000; // 32MB - well above kernel at 1MB

/// Initial heap size (32MB) - large enough for GUI back buffer + TLS + browser state
pub const HEAP_SIZE: usize = 32 * 1024 * 1024;

/// Maximum heap size (128MB)
pub const MAX_HEAP_SIZE: usize = 128 * 1024 * 1024;

/// Global allocator
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Current heap end
static mut HEAP_END: u64 = HEAP_START;

/// Initialize heap allocator
///
/// The heap region HEAP_START..HEAP_START+HEAP_SIZE is identity mapped by the
/// boot page tables (2MB huge pages), so no per-page mapping is needed. The
/// physical range is reserved in the frame allocator during physical::init so
/// it can never be handed out for DMA buffers — remapping it per-page here
/// (the old approach) silently failed against the huge pages and left the
/// heap overlapping "free" physical frames, which corrupted NIC DMA rings.
pub fn init() {
    unsafe {
        HEAP_END = HEAP_START + HEAP_SIZE as u64;
        ALLOCATOR.lock().init(HEAP_START as *mut u8, HEAP_SIZE);
    }
}

/// Extend heap by given size
pub fn extend_heap(additional: usize) -> Result<(), &'static str> {
    unsafe {
        if HEAP_END - HEAP_START + additional as u64 > MAX_HEAP_SIZE as u64 {
            return Err("Maximum heap size exceeded");
        }
        
        let num_pages = (additional + PAGE_SIZE - 1) / PAGE_SIZE;

        // Heap growth continues through the identity-mapped region directly
        // above the current heap; reserve those physical frames so the frame
        // allocator never reuses them for DMA.
        physical::reserve_range(HEAP_END, HEAP_END + (num_pages * PAGE_SIZE) as u64);

        ALLOCATOR.lock().extend(num_pages * PAGE_SIZE);
        HEAP_END += (num_pages * PAGE_SIZE) as u64;

        Ok(())
    }
}

/// Get heap statistics
pub fn heap_stats() -> (usize, usize) {
    let allocator = ALLOCATOR.lock();
    (allocator.free(), allocator.used())
}

/// Get heap size
pub fn heap_size() -> usize {
    unsafe { (HEAP_END - HEAP_START) as usize }
}
