use ariel_os::hal::{i2c, peripherals};

#[cfg(any(context = "nrf52833", context = "nrf52840"))]
pub type SensorI2c = i2c::controller::TWISPI0;
#[cfg(context = "bbc-microbit-v2")]
ariel_os::hal::define_peripherals!(Peripherals {
    i2c_sda: P0_16,
    i2c_scl: P0_08,
});

#[cfg(context = "st-nucleo-f042k6")]
pub type SensorI2c = i2c::controller::I2C1;
#[cfg(context = "st-nucleo-f042k6")]
ariel_os::hal::define_peripherals!(Peripherals {
    i2c_scl: PB6,
    i2c_sda: PB7
});

// This is the I2C bus that the onboard sensors are connected to.
#[cfg(context = "st-steval-mkboxpro")]
pub type SensorI2c = i2c::controller::I2C1;
#[cfg(context = "st-steval-mkboxpro")]
ariel_os::hal::define_peripherals!(Peripherals {
    i2c_scl: PB6,
    i2c_sda: PB7
});

#[cfg(context = "heltec-v3")]
pub type SensorI2c = i2c::controller::I2C1;
#[cfg(context = "heltec-v3")]
ariel_os::hal::define_peripherals!(Peripherals {
    i2c_scl: GPIO18,
    i2c_sda: GPIO17,
});
