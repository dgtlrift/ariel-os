//! The executor: Embassy's raw executor run in lockstep with the simulation.
//!
//! It polls until nothing is ready, then tells the kernel when the next timer is due and
//! waits for the kernel to say the node may go on (`idle`, `advance`). Code runs in no
//! virtual time; only the kernel moves the clock.

#![expect(unsafe_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_executor::{Spawner, raw};
use static_cell::StaticCell;

use crate::kernel;

/// Set when a task was woken and the executor has work.
static PENDING: AtomicBool = AtomicBool::new(false);

#[unsafe(export_name = "__pender")]
fn pender(_context: *mut ()) {
    PENDING.store(true, Ordering::SeqCst);
}

/// The node's executor.
pub struct Executor {
    _private: (),
}

impl Executor {
    /// The executor; [`Executor::run`] starts it.
    #[expect(
        clippy::new_without_default,
        reason = "a const static, as on other HALs"
    )]
    #[must_use]
    pub const fn new() -> Self {
        Self { _private: () }
    }

    /// Runs `init` with a spawner, then the node's tasks for the rest of the process.
    ///
    /// # Panics
    ///
    /// If called twice.
    pub fn run(&'static self, init: impl FnOnce(Spawner)) -> ! {
        static RAW: StaticCell<raw::Executor> = StaticCell::new();
        let raw: &'static raw::Executor = RAW.init(raw::Executor::new(core::ptr::null_mut()));
        init(raw.spawner());
        PENDING.store(true, Ordering::SeqCst);
        loop {
            while PENDING.swap(false, Ordering::SeqCst) {
                // SAFETY: `poll` is called from this one thread only, never from the pender,
                // and on an executor that lives for the rest of the process.
                unsafe { raw.poll() };
            }
            // Wakes the timers already due; the rest give the next wake.
            let next = kernel::with(|k| {
                let now = k.ticks();
                k.timers.next_expiration(now)
            });
            if PENDING.load(Ordering::SeqCst) {
                continue;
            }
            kernel::with(|k| k.idle(next));
            PENDING.store(true, Ordering::SeqCst);
        }
    }
}
