//! Cooperative 2-task system: GUI task (main thread) ↔ Network task.
//!
//! The network task calls `yield_to_main()` at every poll iteration so the
//! GUI renders a fresh frame while waiting for network data — no more freezing.
//!
//! Context switch: saves the 6 x86-64 callee-saved registers on the current
//! stack, swaps RSP, restores the other task's registers, returns.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Dedicated stack for the network task (128 KB — enough for TLS + HTTP).
const NET_STACK_SIZE: usize = 131072;

// These statics are read/written from global_asm, so they must be exported.
#[used] static mut NET_STACK: [u8; NET_STACK_SIZE] = [0u8; NET_STACK_SIZE];
#[used] static mut MAIN_RSP:  u64 = 0;
#[used] static mut NET_RSP:   u64 = 0;

// ── Intel-syntax assembly (LLVM integrated assembler on macOS defaults to Intel)
//
//  _ctx_switch(save: *mut u64 [rdi], load: *const u64 [rsi])
//      Saves callee-saved regs + RSP to *save, loads RSP from *load,
//      restores regs, returns to wherever the other task was.
//
//  _ctx_start_net(net_top: u64 [rdi], entry: u64 [rsi], main_save: *mut u64 [rdx])
//      First-time switch to the net task. Sets up a fresh stack so that
//      the final `ret` lands on `entry` (= net_task_main).
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    // switch to intel syntax explicitly — works on both Linux (GNU as) and macOS (LLVM as)
    ".intel_syntax noprefix",

    ".global _ctx_switch",
    "_ctx_switch:",
    "push rbp",
    "push rbx",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov [rdi], rsp",       // save current RSP to *rdi
    "mov rsp, [rsi]",       // load other task's RSP from *rsi
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbx",
    "pop rbp",
    "ret",

    ".global _ctx_start_net",
    "_ctx_start_net:",
    // save main task's regs + RSP
    "push rbp",
    "push rbx",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "mov [rdx], rsp",       // save main RSP
    // switch to net stack
    "mov rsp, rdi",         // rsp = net_top (16-byte aligned minus 8)
    "push rsi",             // push entry as the fake return address
    // zero-init the net task's saved registers
    "push 0",               // r15 = 0
    "push 0",               // r14 = 0
    "push 0",               // r13 = 0
    "push 0",               // r12 = 0
    "push 0",               // rbx = 0
    "push 0",               // rbp = 0
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbx",
    "pop rbp",
    "ret",                  // pops `entry` off the net stack → jumps to net_task_main

    ".att_syntax prefix",   // restore default syntax for anything that follows
);

#[cfg(target_arch = "x86_64")]
extern "C" {
    fn _ctx_switch(save_rsp: *mut u64, load_rsp: *const u64);
    fn _ctx_start_net(net_top: u64, entry: u64, main_save: *mut u64);
}

// ── task state ────────────────────────────────────────────────────────────────

static NET_RUNNING: AtomicBool = AtomicBool::new(false);
static NET_DONE:    AtomicBool = AtomicBool::new(false);
static NET_WIN_ID:  AtomicU32  = AtomicU32::new(0);

static mut PENDING_URL:   Option<String> = None;
static mut FETCH_RESULT:  Option<Result<(String, Vec<String>), String>> = None;

// ── net task entry ────────────────────────────────────────────────────────────

/// Runs inside the network task. Calls `fetch_and_render`, stores the result,
/// then switches back to the main task permanently.
#[cfg(target_arch = "x86_64")]
unsafe extern "C" fn net_task_main() {
    let url = PENDING_URL.take().unwrap_or_default();
    let result = crate::browser::fetch_and_render(&url);
    FETCH_RESULT = Some(result);
    NET_DONE.store(true, Ordering::SeqCst);
    // Final switch back to main (net task is finished)
    _ctx_switch(&mut NET_RSP, &MAIN_RSP);
    loop { crate::arch::halt(); } // unreachable
}

// ── public API ────────────────────────────────────────────────────────────────

/// Returns `true` while a network fetch is in progress.
pub fn is_running() -> bool {
    NET_RUNNING.load(Ordering::SeqCst)
}

/// Start a network fetch for `url` on behalf of browser window `win_id`.
/// Returns after the net task's first `yield_to_main()` call.
/// Then call `tick_network()` each GUI frame and `take_result()` when done.
pub fn start_fetch(url: String, win_id: u32) {
    if NET_RUNNING.load(Ordering::SeqCst) { return; }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        PENDING_URL = Some(url);
        NET_DONE.store(false, Ordering::SeqCst);
        NET_RUNNING.store(true, Ordering::SeqCst);
        NET_WIN_ID.store(win_id, Ordering::SeqCst);

        // Stack top: 16-byte aligned, then -8 for the ABI call convention
        let raw_top = NET_STACK.as_ptr().add(NET_STACK_SIZE) as u64;
        let net_top = (raw_top & !15u64) - 8;

        _ctx_start_net(net_top, net_task_main as u64, &mut MAIN_RSP);
        // Returns here after net task's first yield_to_main()
    }
}

/// Give the net task one cooperative slice. Returns `true` when done.
/// Call once per GUI frame while `is_running()` is true.
pub fn tick_network() -> bool {
    if NET_DONE.load(Ordering::SeqCst) { return true; }
    if NET_RUNNING.load(Ordering::SeqCst) {
        #[cfg(target_arch = "x86_64")]
        unsafe { _ctx_switch(&mut MAIN_RSP, &NET_RSP); }
        return NET_DONE.load(Ordering::SeqCst);
    }
    false
}

/// Take the result after `tick_network()` returns `true`.
/// Returns `(win_id, Option<result>)`.
pub fn take_result() -> (u32, Option<Result<(String, Vec<String>), String>>) {
    NET_RUNNING.store(false, Ordering::SeqCst);
    NET_DONE.store(false, Ordering::SeqCst);
    let win_id = NET_WIN_ID.load(Ordering::SeqCst);
    (win_id, unsafe { FETCH_RESULT.take() })
}

/// Call from inside the network task instead of `crate::arch::halt()`.
/// Yields to the GUI for one frame, then returns.
pub fn yield_to_main() {
    if NET_RUNNING.load(Ordering::SeqCst) {
        #[cfg(target_arch = "x86_64")]
        unsafe { _ctx_switch(&mut NET_RSP, &MAIN_RSP); }
    } else {
        crate::arch::halt();
    }
}
