//! Provides support for the I2C communication bus.

#[doc(alias = "master")]
pub mod controller;

#[doc(hidden)]
pub fn init(peripherals: &mut crate::OptionalPeripherals) {
    // Drivers are created from the peripheral type; keep the singleton from other uses.
    let _ = peripherals.I2C0.take();
}
