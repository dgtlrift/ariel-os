//! BLE: the simulator's HCI controller (`ble-hci`) on UART1, through `bt-hci`'s UART
//! transport (H4), so `trouble-host` runs on it as on any other controller.
//!
//! The controller's link layer, advertising and scanning on the fabric, is the
//! simulator's model; this HAL only carries HCI to and from it.

use core::convert::Infallible;

use bt_hci::controller::ExternalController;
use bt_hci::transport::SerialTransport;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;

#[cfg(not(context = "nrf52840"))]
use crate::gpio::Pin;
use crate::uart;
use crate::wire::UartOpenParity;

/// Number of command slots for the BLE driver.
pub const SLOTS: usize = 10;

/// The HCI UART's baud rate: the simulated controller's (`ble-hci`).
const HCI_BAUD: u32 = 1_000_000;
/// The receive buffer for HCI events.
const HCI_RX_BUFFER: usize = 4096;

/// The controller's events, from the HCI UART.
pub struct HciRx(&'static str);
/// The host's commands, to the HCI UART.
pub struct HciTx(&'static str);

impl embedded_io_async::ErrorType for HciRx {
    type Error = Infallible;
}

impl embedded_io_async::Read for HciRx {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        Ok(uart::read(self.0, buf).await)
    }
}

impl embedded_io_async::ErrorType for HciTx {
    type Error = Infallible;
}

impl embedded_io_async::Write for HciTx {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "the kernel answers before the request returns (lockstep)"
    )]
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        Ok(uart::write(self.0, buf))
    }

    #[expect(
        clippy::unused_async_trait_impl,
        reason = "a write returns when its last byte has left"
    )]
    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// The BLE controller: the simulator's, behind HCI over UART1.
pub type BleController = ExternalController<SerialTransport<NoopRawMutex, HciRx, HciTx>, SLOTS>;

/// The HCI line of a chip whose radio is inside it (D40): no board UART or pins, so the
/// simulator's controller sits where the chip's link layer would.
#[cfg(context = "nrf52840")]
const INTERNAL_HCI: (&str, &str) = ("HCI_RX", "HCI_TX");

/// Opens the chip's internal HCI line at 1 Mbaud, 8N1, to the controller (D40).
///
/// # Panics
///
/// If the simulator has no BLE controller on it.
#[cfg(context = "nrf52840")]
#[must_use]
pub fn build_controller(
    _peripherals: &mut crate::OptionalPeripherals,
    _spawner: Spawner,
) -> BleController {
    let (rx, tx) = INTERNAL_HCI;
    uart::open(tx, rx, HCI_RX_BUFFER, HCI_BAUD, 8, UartOpenParity::None, 1)
        .unwrap_or_else(|e| panic!("ariel-sim: the chip has no BLE controller: {e:?}"));
    ExternalController::new(SerialTransport::new(HciRx(rx), HciTx(tx)))
}

/// Opens UART1 (RX `P0_16`, TX `P0_17`) at 1 Mbaud, 8N1, to the controller.
///
/// # Panics
///
/// If the board has no BLE controller on UART1, or UART1 was taken.
#[cfg(not(context = "nrf52840"))]
#[must_use]
pub fn build_controller(
    peripherals: &mut crate::OptionalPeripherals,
    _spawner: Spawner,
) -> BleController {
    let taken = (
        peripherals.UART1.take(),
        peripherals.P0_16.take(),
        peripherals.P0_17.take(),
    );
    assert!(
        taken.0.is_some() && taken.1.is_some() && taken.2.is_some(),
        "BLE needs UART1 and its pins P0_16 and P0_17"
    );
    let (rx, tx) = (
        <crate::peripherals::P0_16 as Pin>::NAME,
        <crate::peripherals::P0_17 as Pin>::NAME,
    );
    uart::open(tx, rx, HCI_RX_BUFFER, HCI_BAUD, 8, UartOpenParity::None, 1)
        .unwrap_or_else(|e| panic!("ariel-sim: the board has no BLE controller on UART1: {e:?}"));
    ExternalController::new(SerialTransport::new(HciRx(rx), HciTx(tx)))
}
