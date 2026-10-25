//! Provides support for the I2C communication bus in controller mode.
//!
//! Each transaction is one `i2c` request to the kernel: a start, the operations with
//! repeated starts, a stop, at the clock rate the firmware configured.

use crate::wire::generated::I2cMsg;
use crate::wire::{I2c as I2cRequest, I2cOp, I2cOpKind, NodeMessage};
use ariel_os_embassy_common::i2c::controller::{Error, NoAcknowledgeSource};
use ariel_os_embassy_common::impl_i2c_from_frequency;
use embedded_hal::i2c::Operation;

use crate::gpio::Pin;
use crate::kernel;

pub use ariel_os_embassy_common::i2c::controller::Kilohertz;

/// I2C bus configuration.
#[derive(Clone)]
#[non_exhaustive]
pub struct Config {
    /// The frequency at which the bus should operate.
    pub frequency: Frequency,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            frequency: Frequency::_100k,
        }
    }
}

/// I2C bus frequency.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Frequency {
    /// Standard mode.
    _100k,
    /// Fast mode.
    _400k,
}

#[doc(hidden)]
impl Frequency {
    #[must_use]
    pub const fn first() -> Self {
        Self::_100k
    }

    #[must_use]
    pub const fn last() -> Self {
        Self::_400k
    }

    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self {
            Self::_100k => Some(Self::_400k),
            Self::_400k => None,
        }
    }

    #[must_use]
    pub const fn prev(self) -> Option<Self> {
        match self {
            Self::_100k => None,
            Self::_400k => Some(Self::_100k),
        }
    }

    #[must_use]
    pub const fn khz(self) -> u32 {
        match self {
            Self::_100k => 100,
            Self::_400k => 400,
        }
    }
}

impl_i2c_from_frequency!();

/// The `I2C0` peripheral's driver.
pub struct I2C0 {
    hz: u64,
}

impl I2C0 {
    /// Returns a driver implementing [`embedded_hal_async::i2c::I2c`] for this I2C
    /// peripheral.
    #[expect(clippy::new_ret_no_self)]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the shape every Ariel HAL's driver has"
    )]
    #[must_use]
    pub fn new<SDA: Pin, SCL: Pin>(
        sda_pin: impl crate::IntoPeripheral<'static, SDA>,
        scl_pin: impl crate::IntoPeripheral<'static, SCL>,
        config: Config,
    ) -> I2c {
        let _ = (sda_pin.into_hal_peripheral(), scl_pin.into_hal_peripheral());
        I2c::I2C0(Self {
            hz: u64::from(config.frequency.khz()) * 1000,
        })
    }
}

/// Peripheral-agnostic driver.
pub enum I2c {
    /// I2C0 peripheral.
    I2C0(I2C0),
}

impl embedded_hal_async::i2c::ErrorType for I2c {
    type Error = Error;
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the kernel answers synchronously (lockstep)"
)]
impl embedded_hal_async::i2c::I2c for I2c {
    async fn transaction(
        &mut self,
        address: u8,
        operations: &mut [Operation<'_>],
    ) -> Result<(), Self::Error> {
        let Self::I2C0(i2c) = self;
        let ops = operations
            .iter()
            .map(|op| match op {
                Operation::Read(buf) => I2cOp {
                    kind: I2cOpKind::Read,
                    out: None,
                    len: Some(buf.len() as u64),
                },
                Operation::Write(buf) => I2cOp {
                    kind: I2cOpKind::Write,
                    out: Some(buf.to_vec()),
                    len: None,
                },
            })
            .collect();
        let reply = kernel::with(|k| {
            k.request(&NodeMessage::I2c(I2cRequest {
                msg: I2cMsg,
                t: k.now(),
                bus: "I2C0".into(),
                hz: i2c.hz,
                address: u64::from(address),
                ops,
            }))
        });
        if let Some(byte) = reply.nack {
            let source = if byte == 0 {
                NoAcknowledgeSource::Address
            } else {
                NoAcknowledgeSource::Data
            };
            return Err(Error::NoAcknowledge(source));
        }
        // The reads' bytes, concatenated in order.
        let mut data = reply.data.unwrap_or_default().into_iter();
        for op in operations {
            if let Operation::Read(buf) = op {
                for b in buf.iter_mut() {
                    *b = data.next().unwrap_or(0xFF);
                }
            }
        }
        Ok(())
    }
}
