//! Items specific to `sim-chip`: Ariel OS firmware running as a host process inside the
//! ariel-sim simulator ([dgtlrift/ariel-sim](https://github.com/dgtlrift/ariel-sim)).
//!
//! The peripherals are the simulator's device models. Every bus operation, pin change and
//! flash operation is a message to the simulator's kernel over a Unix socket, answered when
//! the operation ends in virtual time. The executor runs in lockstep with the simulation,
//! so a run replays exactly. A binary built for `-b sim` is started by the simulator, not
//! by hand.

#![deny(missing_docs)]

mod executor;
mod kernel;
// A copy of ariel-sim's sim-node-wire crate (scripts/sync-wire.sh there); never edited.
#[cfg(feature = "time")]
mod time;
#[allow(
    clippy::all,
    clippy::pedantic,
    clippy::unused_trait_names,
    dead_code,
    unreachable_pub,
    unused_imports,
    reason = "copied from ariel-sim, which lints it there"
)]
#[rustfmt::skip] // byte for byte ariel-sim's, formatted there
mod wire;

#[cfg(feature = "ble")]
pub mod ble;

pub mod gpio;

#[cfg(feature = "hwrng")]
#[doc(hidden)]
pub mod hwrng;

#[cfg(feature = "i2c")]
pub mod i2c;

#[doc(hidden)]
pub mod identity;

#[cfg(feature = "spi")]
pub mod spi;

#[cfg(feature = "storage")]
#[doc(hidden)]
pub mod storage;

#[cfg(feature = "uart")]
pub mod uart;

#[doc(hidden)]
pub use executor::Executor;

#[doc(hidden)]
pub mod peripheral {
    pub use embassy_hal_internal::Peri;
}

use embassy_hal_internal::{Peri, PeripheralType};

/// Defines the peripheral types and the [`OptionalPeripherals`] holding one of each.
macro_rules! sim_peripherals {
    ($($name:ident),* $(,)?) => {
        #[allow(clippy::expl_impl_clone_on_copy, reason = "embassy-hal-internal's macro")]
        mod defs {
            embassy_hal_internal::peripherals_definition!($($name),*);
        }
        pub use defs::peripherals;

        /// The peripherals not yet taken by a driver.
        #[allow(non_snake_case, missing_docs)]
        pub struct OptionalPeripherals {
            $(pub $name: Option<Peri<'static, peripherals::$name>>,)*
        }

        impl OptionalPeripherals {
            /// Every peripheral, once per process.
            #[expect(unsafe_code)]
            fn all() -> Self {
                static TAKEN: core::sync::atomic::AtomicBool =
                    core::sync::atomic::AtomicBool::new(false);
                assert!(
                    !TAKEN.swap(true, core::sync::atomic::Ordering::SeqCst),
                    "hal::init() called twice"
                );
                Self {
                    // SAFETY: the guard above makes this the only instance of each.
                    $($name: Some(unsafe { peripherals::$name::steal() }),)*
                }
            }
        }
    };
}

sim_peripherals!(
    P0_00, P0_01, P0_02, P0_03, P0_04, P0_05, P0_06, P0_07, P0_08, P0_09, P0_10, P0_11, P0_12,
    P0_13, P0_14, P0_15, P0_16, P0_17, P0_18, P0_19, P0_20, P0_21, P0_22, P0_23, P0_24, P0_25,
    P0_26, P0_27, P0_28, P0_29, P0_30, P0_31, SPI0, I2C0, FLASH, UART0, UART1,
);

/// The executor of this node.
#[doc(hidden)]
pub static EXECUTOR: Executor = Executor::new();

/// The software interrupt other HALs start their executor on; unused here.
#[doc(hidden)]
pub struct SWI;

#[doc(hidden)]
pub trait IntoPeripheral<'a, T: PeripheralType>: private::Sealed {
    fn into_hal_peripheral(self) -> Peri<'a, T>;
}

impl<T: PeripheralType> private::Sealed for Peri<'_, T> {}

#[doc(hidden)]
impl<'a, T: PeripheralType> IntoPeripheral<'a, T> for Peri<'a, T> {
    fn into_hal_peripheral(self) -> Peri<'a, T> {
        self
    }
}

mod private {
    pub trait Sealed {}
}

/// Connects to the simulator, takes this node's start message, and returns the
/// peripherals.
#[doc(hidden)]
#[must_use]
pub fn init() -> OptionalPeripherals {
    // A panic can't unwind out of the host `main` (an `extern "C"` function), so the
    // default would abort with a second panic and a backtrace that hides the first. Print
    // the panic and exit with Rust's panic status instead; the simulator traces both.
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write as _;
        eprintln!("{info}");
        let _ = std::io::stderr().flush();
        std::process::exit(101);
    }));
    kernel::connect();
    OptionalPeripherals::all()
}

/// This boot's number, counting from 1 for the first boot of a run.
#[must_use]
pub fn boot_count() -> u64 {
    kernel::with(|k| k.identity.boot)
}

/// The simulator's id of this node.
#[must_use]
pub fn node_id() -> u64 {
    kernel::with(|k| k.identity.node)
}
