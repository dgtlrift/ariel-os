//! The time driver: the node's virtual clock at 1 MHz (D8).

use core::task::Waker;

use embassy_time_driver::Driver;

use crate::kernel;

struct SimTime;

impl Driver for SimTime {
    fn now(&self) -> u64 {
        kernel::with(|k| k.ticks())
    }

    fn schedule_wake(&self, at: u64, waker: &Waker) {
        kernel::with(|k| {
            k.timers.schedule_wake(at, waker);
        });
    }
}

embassy_time_driver::time_driver_impl!(static DRIVER: SimTime = SimTime);
