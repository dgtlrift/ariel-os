//! The executor: Embassy's raw executor run in lockstep with the simulation.
//!
//! It polls until nothing is ready, then tells the kernel when the next timer is due and
//! waits for the kernel to say the node may go on (`idle`, `advance`). Code runs in no
//! virtual time; only the kernel moves the clock.
//!
//! Some code polls by waking itself until something else happens (`embassy_sync`'s
//! `OnceLock::get`, which `trouble-host` waits on), which on hardware spins while time
//! passes. Here a task that keeps the executor busy without ever asking the kernel for
//! anything would stop virtual time. So after [`SPIN_PASSES`] passes in a row that left
//! the node's clock where it was, one tick of virtual time passes, as an `idle` with the
//! next wake a tick away (M8c).

#![expect(unsafe_code)]

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_executor::{Spawner, raw};
use static_cell::StaticCell;

use crate::kernel;

/// Set when a task was woken and the executor has work.
static PENDING: AtomicBool = AtomicBool::new(false);

/// Passes in a row that left the node's clock where it was, after which a tick passes.
/// Tasks waking each other take a few passes; only a task polling by waking itself takes
/// this many.
const SPIN_PASSES: u32 = 1000;

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
        // Passes in a row that left the node's clock where it was.
        let mut still = 0;
        loop {
            let mut spinning = false;
            while PENDING.swap(false, Ordering::SeqCst) {
                let before = kernel::with(|k| k.now());
                // SAFETY: `poll` is called from this one thread only, never from the pender,
                // and on an executor that lives for the rest of the process.
                unsafe { raw.poll() };
                if kernel::with(|k| k.now()) == before {
                    still += 1;
                } else {
                    still = 0;
                }
                if still >= SPIN_PASSES && PENDING.load(Ordering::SeqCst) {
                    spinning = true;
                    break;
                }
            }
            // Wakes the timers already due; the rest give the next wake.
            let next = kernel::with(|k| {
                let now = k.ticks();
                let next = k.timers.next_expiration(now);
                if spinning { next.min(now + 1) } else { next }
            });
            if PENDING.load(Ordering::SeqCst) && !spinning {
                continue;
            }
            still = 0;
            kernel::with(|k| k.idle(next));
            PENDING.store(true, Ordering::SeqCst);
        }
    }
}
