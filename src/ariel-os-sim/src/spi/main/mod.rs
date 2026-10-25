//! Provides support for the SPI communication bus in main mode.
//!
//! Each `read`, `write`, `transfer` and `transfer_in_place` is one `spi` request to the
//! kernel, at the clock rate the firmware configured, so the bus time is the one a real
//! bus at that rate takes. The device answering is the one whose chip select is low.

use crate::wire::generated::SpiMsg;
use crate::wire::{NodeMessage, Spi as SpiRequest, SpiOp, SpiOpKind};
use ariel_os_embassy_common::spi::{BitOrder, Mode};
use ariel_os_embassy_common::{impl_spi_frequency_const_functions, impl_spi_from_frequency};

use crate::gpio::Pin;
use crate::kernel;

pub use ariel_os_embassy_common::spi::main::Kilohertz;

const MAX_FREQUENCY: Kilohertz = Kilohertz::MHz(8);

/// SPI bus configuration.
#[derive(Clone)]
#[non_exhaustive]
pub struct Config {
    /// The frequency at which the bus should operate.
    pub frequency: Frequency,
    /// The SPI mode to use (the simulated devices don't depend on it).
    pub mode: Mode,
    #[doc(hidden)]
    pub bit_order: BitOrder,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            frequency: Frequency::F(Kilohertz::MHz(1)),
            mode: Mode::Mode0,
            bit_order: BitOrder::default(),
        }
    }
}

/// SPI bus frequency.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Frequency {
    #[doc(hidden)]
    F(Kilohertz),
}

impl_spi_frequency_const_functions!(MAX_FREQUENCY);
impl_spi_from_frequency!();

/// The `SPI0` peripheral's driver.
pub struct SPI0 {
    hz: u64,
}

impl SPI0 {
    /// Returns a driver implementing [`embedded_hal_async::spi::SpiBus`] for this SPI
    /// peripheral.
    #[expect(clippy::new_ret_no_self)]
    #[must_use]
    pub fn new<SCK: Pin, MISO: Pin, MOSI: Pin>(
        sck_pin: impl crate::IntoPeripheral<'static, SCK>,
        miso_pin: impl crate::IntoPeripheral<'static, MISO>,
        mosi_pin: impl crate::IntoPeripheral<'static, MOSI>,
        config: Config,
    ) -> Spi {
        let _ = (
            sck_pin.into_hal_peripheral(),
            miso_pin.into_hal_peripheral(),
            mosi_pin.into_hal_peripheral(),
        );
        Spi::SPI0(Self {
            hz: u64::from(config.frequency.khz()) * 1000,
        })
    }

    /// One operation, split so each request stays under the message limit; returns the
    /// bytes clocked in, as many as were clocked out (or `len` for a read).
    fn exchange(&self, kind: SpiOpKind, out: Option<&[u8]>, len: usize) -> Vec<u8> {
        let mut data = Vec::with_capacity(len);
        let mut at = 0;
        while at < len {
            let n = (len - at).min(crate::wire::MAX_CHUNK);
            let op = SpiOp {
                kind: kind.clone(),
                out: out.and_then(|o| o.get(at..at + n)).map(<[u8]>::to_vec),
                len: out.is_none().then_some(n as u64),
            };
            let reply = kernel::with(|k| {
                k.request(&NodeMessage::Spi(SpiRequest {
                    msg: SpiMsg,
                    t: k.now(),
                    bus: "SPI0".into(),
                    hz: self.hz,
                    op,
                }))
            });
            let mut chunk = reply.data.unwrap_or_default();
            chunk.resize(n, 0);
            data.extend_from_slice(&chunk);
            at += n;
        }
        data
    }
}

/// Peripheral-agnostic driver.
pub enum Spi {
    /// SPI0 peripheral.
    SPI0(SPI0),
}

impl embedded_hal_async::spi::ErrorType for Spi {
    type Error = core::convert::Infallible;
}

impl embedded_hal_async::spi::SpiBus for Spi {
    async fn read(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        let Self::SPI0(spi) = self;
        let data = spi.exchange(SpiOpKind::Read, None, words.len());
        words.copy_from_slice(&data);
        Ok(())
    }

    async fn write(&mut self, words: &[u8]) -> Result<(), Self::Error> {
        let Self::SPI0(spi) = self;
        spi.exchange(SpiOpKind::Write, Some(words), words.len());
        Ok(())
    }

    async fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        let Self::SPI0(spi) = self;
        // The bus clocks the longer side; past `write`, the host sends 0x00.
        let mut out = write.to_vec();
        out.resize(read.len().max(write.len()), 0);
        let data = spi.exchange(SpiOpKind::Transfer, Some(&out), out.len());
        for (r, d) in read.iter_mut().zip(data) {
            *r = d;
        }
        Ok(())
    }

    async fn transfer_in_place(&mut self, words: &mut [u8]) -> Result<(), Self::Error> {
        let Self::SPI0(spi) = self;
        let data = spi.exchange(SpiOpKind::Transfer, Some(words), words.len());
        words.copy_from_slice(&data);
        Ok(())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
