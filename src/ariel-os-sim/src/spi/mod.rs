//! Provides support for the SPI communication bus.

#[doc(alias = "master")]
pub mod main;

#[doc(hidden)]
pub fn init(peripherals: &mut crate::OptionalPeripherals) {
    // Drivers are created from the peripheral type; keep the singleton from other uses.
    let _ = peripherals.SPI0.take();
}
