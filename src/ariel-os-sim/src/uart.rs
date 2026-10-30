//! Provides support for UART.
//!
//! A UART is the simulator's: opening it sends `uart-open` with its pins and framing,
//! each write is one `uart-write` answered when the last byte has left (so it takes the
//! line's time at the configured baud rate), and what the device on the far end sends
//! arrives with the kernel's `advance` and `reply`, each chunk at the time its last byte
//! did. Bytes beyond the receive buffer's size are lost, as on a real overrun.

use core::convert::Infallible;
use core::marker::PhantomData;
use core::task::Poll;

use ariel_os_embassy_common::{impl_async_uart_for_driver_enum, uart::ConfigError};

use crate::gpio::Pin;
use crate::kernel;
use crate::wire::generated::{UartOpenMsg, UartWriteMsg};
use crate::wire::{MAX_CHUNK, NodeMessage, UartOpen, UartOpenParity, UartWrite};

/// UART interface configuration.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// The baud rate at which UART operates.
    pub baudrate: ariel_os_embassy_common::uart::Baudrate<Baudrate>,
    /// Number of data bits.
    pub data_bits: DataBits,
    /// Number of stop bits.
    pub stop_bits: StopBits,
    /// Parity mode used for the interface.
    pub parity: Parity,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            baudrate: ariel_os_embassy_common::uart::Baudrate::_115200,
            data_bits: DataBits::Data8,
            stop_bits: StopBits::Stop1,
            parity: Parity::None,
        }
    }
}

/// UART baud rate: any the simulated line runs at.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Baudrate {
    /// 230400 baud.
    _230400,
    /// 460800 baud.
    _460800,
    /// 921600 baud.
    _921600,
    /// 1 Megabaud.
    _1000000,
    /// Any other rate, in baud.
    Other(u32),
}

impl From<Baudrate> for u32 {
    fn from(baud: Baudrate) -> u32 {
        match baud {
            Baudrate::_230400 => 230_400,
            Baudrate::_460800 => 460_800,
            Baudrate::_921600 => 921_600,
            Baudrate::_1000000 => 1_000_000,
            Baudrate::Other(b) => b,
        }
    }
}

fn baud(b: ariel_os_embassy_common::uart::Baudrate<Baudrate>) -> u32 {
    use ariel_os_embassy_common::uart::Baudrate as B;
    match b {
        B::Hal(b) => b.into(),
        B::_2400 => 2400,
        B::_4800 => 4800,
        B::_9600 => 9600,
        B::_19200 => 19_200,
        B::_38400 => 38_400,
        B::_57600 => 57_600,
        B::_115200 => 115_200,
    }
}

/// UART number of data bits.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DataBits {
    /// 7 bits per character.
    Data7,
    /// 8 bits per character.
    Data8,
}

impl From<ariel_os_embassy_common::uart::DataBits<Self>> for DataBits {
    fn from(bits: ariel_os_embassy_common::uart::DataBits<Self>) -> DataBits {
        match bits {
            ariel_os_embassy_common::uart::DataBits::Hal(bits) => bits,
            ariel_os_embassy_common::uart::DataBits::Data8 => DataBits::Data8,
        }
    }
}

/// UART number of stop bits.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StopBits {
    /// One stop bit.
    Stop1,
    /// Two stop bits.
    Stop2,
}

impl From<ariel_os_embassy_common::uart::StopBits<Self>> for StopBits {
    fn from(stop: ariel_os_embassy_common::uart::StopBits<Self>) -> Self {
        match stop {
            ariel_os_embassy_common::uart::StopBits::Hal(stop) => stop,
            ariel_os_embassy_common::uart::StopBits::Stop1 => StopBits::Stop1,
        }
    }
}

/// Parity bit.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Parity {
    /// No parity bit.
    None,
    /// Even parity bit.
    Even,
    /// Odd parity bit.
    Odd,
}

impl From<ariel_os_embassy_common::uart::Parity<Self>> for Parity {
    fn from(parity: ariel_os_embassy_common::uart::Parity<Self>) -> Self {
        match parity {
            ariel_os_embassy_common::uart::Parity::Hal(parity) => parity,
            ariel_os_embassy_common::uart::Parity::None => Self::None,
            ariel_os_embassy_common::uart::Parity::Even => Self::Even,
        }
    }
}

/// Opens the simulator's UART on these pins (`uart-open`) and starts receiving into a
/// buffer of `capacity` bytes.
///
/// # Errors
///
/// [`ConfigError::ConfigurationNotSupported`] if the board has no UART on these pins
/// or the line can't take the framing.
pub(crate) fn open(
    tx: &'static str,
    rx: &'static str,
    capacity: usize,
    baud: u32,
    data_bits: u64,
    parity: UartOpenParity,
    stop_bits: u64,
) -> Result<(), ConfigError> {
    kernel::with(|k| {
        let msg = NodeMessage::UartOpen(UartOpen {
            msg: UartOpenMsg,
            t: k.now(),
            tx: tx.into(),
            rx: rx.into(),
            baud: u64::from(baud),
            data_bits,
            parity,
            stop_bits,
        });
        k.try_request(&msg)
            .map_err(|_| ConfigError::ConfigurationNotSupported)?;
        k.uart_opened(rx, capacity);
        Ok(())
    })
}

/// Reads what has arrived on `rx`, waiting for at least one byte.
pub(crate) async fn read(rx: &'static str, buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    core::future::poll_fn(|cx| {
        kernel::with(|k| match k.uart_take(rx, buf) {
            0 => {
                k.wake_on_uart(rx, cx.waker());
                Poll::Pending
            }
            n => Poll::Ready(n),
        })
    })
    .await
}

/// Sends up to [`MAX_CHUNK`] bytes on `tx` (`uart-write`): the node's clock moves on by
/// their time on the line. How many were sent.
pub(crate) fn write(tx: &'static str, buf: &[u8]) -> usize {
    let n = buf.len().min(MAX_CHUNK);
    if n == 0 {
        return 0;
    }
    kernel::with(|k| {
        let t = k.now();
        k.request(&NodeMessage::UartWrite(UartWrite {
            msg: UartWriteMsg,
            t,
            tx: tx.into(),
            data: buf.get(..n).unwrap_or_default().to_vec(),
        }));
    });
    n
}

/// One open simulated UART, by its pins.
pub struct SimUart<'d> {
    tx: &'static str,
    rx: &'static str,
    _buffers: PhantomData<&'d mut [u8]>,
}

impl SimUart<'_> {
    /// Opens the UART with the app's configuration.
    ///
    /// # Errors
    ///
    /// As [`open`].
    fn open(
        tx: &'static str,
        rx: &'static str,
        rx_buffer: &mut [u8],
        config: Config,
    ) -> Result<Self, ConfigError> {
        open(
            tx,
            rx,
            rx_buffer.len(),
            baud(config.baudrate),
            match config.data_bits {
                DataBits::Data7 => 7,
                DataBits::Data8 => 8,
            },
            match config.parity {
                Parity::None => UartOpenParity::None,
                Parity::Even => UartOpenParity::Even,
                Parity::Odd => UartOpenParity::Odd,
            },
            match config.stop_bits {
                StopBits::Stop1 => 1,
                StopBits::Stop2 => 2,
            },
        )?;
        Ok(Self {
            tx,
            rx,
            _buffers: PhantomData,
        })
    }
}

impl Drop for SimUart<'_> {
    fn drop(&mut self) {
        kernel::with(|k| k.uart_closed(self.rx));
    }
}

impl embedded_io_async::ErrorType for SimUart<'_> {
    type Error = Infallible;
}

impl embedded_io_async::Read for SimUart<'_> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(read(self.rx, buf).await)
    }
}

impl embedded_io_async::ReadReady for SimUart<'_> {
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(kernel::with(|k| k.uart_available(self.rx)) > 0)
    }
}

impl embedded_io_async::Write for SimUart<'_> {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "the kernel answers before the request returns (lockstep)"
    )]
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(write(self.tx, buf))
    }

    #[expect(
        clippy::unused_async_trait_impl,
        reason = "a write returns when its last byte has left"
    )]
    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

macro_rules! define_uart_drivers {
    ($( $peripheral:ident ),* $(,)?) => {
        $(
            #[doc = concat!("The `", stringify!($peripheral), "` peripheral's driver.")]
            pub struct $peripheral<'d> {
                uart: SimUart<'d>,
            }

            impl<'d> $peripheral<'d> {
                #[doc = concat!("Opens `", stringify!($peripheral), "` on these pins.")]
                #[doc = ""]
                #[doc = "# Errors"]
                #[doc = ""]
                #[doc = "[`ConfigError::ConfigurationNotSupported`] if the board has no UART"]
                #[doc = "on these pins, or the line can't take the configuration."]
                #[expect(clippy::new_ret_no_self)]
                pub fn new<RX: Pin, TX: Pin>(
                    rx_pin: impl crate::IntoPeripheral<'d, RX>,
                    tx_pin: impl crate::IntoPeripheral<'d, TX>,
                    rx_buffer: &'d mut [u8],
                    _tx_buffer: &'d mut [u8],
                    config: Config,
                ) -> Result<Uart<'d>, ConfigError> {
                    let _ = (rx_pin.into_hal_peripheral(), tx_pin.into_hal_peripheral());
                    let uart = SimUart::open(TX::NAME, RX::NAME, rx_buffer, config)?;
                    Ok(Uart::$peripheral(Self { uart }))
                }
            }
        )*

        /// A UART driver of any peripheral.
        pub enum Uart<'d> {
            $(
                #[doc = concat!(stringify!($peripheral), " peripheral.")]
                $peripheral($peripheral<'d>)
            ),*
        }

        impl embedded_io_async::ErrorType for Uart<'_> {
            type Error = Infallible;
        }

        impl_async_uart_for_driver_enum!(Uart, $( $peripheral ),*);
    };
}

#[cfg(sim_chip = "generic")]
define_uart_drivers!(UART0, UART1);
#[cfg(sim_chip = "nrf52840")]
define_uart_drivers!(UARTE0, UARTE1);
#[cfg(sim_chip = "esp32")]
define_uart_drivers!(UART0, UART1, UART2);
#[cfg(sim_chip = "esp32s2")]
define_uart_drivers!(UART0, UART1);
#[cfg(sim_chip = "esp32s3")]
define_uart_drivers!(UART0, UART1, UART2);

#[doc(hidden)]
pub fn init(peripherals: &mut crate::OptionalPeripherals) {
    // Drivers are created from the peripheral type; keep the singletons from other uses.
    #[cfg(sim_chip = "generic")]
    let _ = (peripherals.UART0.take(), peripherals.UART1.take());
    #[cfg(sim_chip = "nrf52840")]
    let _ = (peripherals.UARTE0.take(), peripherals.UARTE1.take());
    #[cfg(sim_chip = "esp32")]
    let _ = (peripherals.UART0.take(), peripherals.UART1.take(), peripherals.UART2.take());
    #[cfg(sim_chip = "esp32s2")]
    let _ = (peripherals.UART0.take(), peripherals.UART1.take());
    #[cfg(sim_chip = "esp32s3")]
    let _ = (peripherals.UART0.take(), peripherals.UART1.take(), peripherals.UART2.take());
}
