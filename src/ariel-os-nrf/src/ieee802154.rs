//! The chip's IEEE 802.15.4 radio: embassy-nrf's `radio::ieee802154` driver, with its
//! interrupt bound here (ariel-sim D60).
//!
//! The same API is the sim HAL's `ieee802154`, so an app written against
//! `ariel_os::hal::ieee802154` builds for an nRF52840 and for the ariel-sim simulator. On
//! the chip, BLE and this driver can't share the radio yet: that needs MPSL's timeslot
//! API, which the simulator models and Ariel doesn't use.

#[cfg(feature = "ble")]
compile_error!("ieee802154 and ble both need the nRF52840's RADIO, and Ariel can't share it yet");

use embassy_nrf::{Peri, peripherals::RADIO, radio::ieee802154};

pub use embassy_nrf::radio::Error;
pub use embassy_nrf::radio::ieee802154::{Cca, DEFAULT_SFD, Packet};

/// IEEE 802.15.4 radio driver: embassy-nrf's, its interrupt bound by the HAL.
pub struct Radio<'d>(ieee802154::Radio<'d>);

impl<'d> Radio<'d> {
    /// Creates a new IEEE 802.15.4 radio driver: channel 11, 0 dBm, carrier-sense CCA, the
    /// standard SFD.
    #[must_use]
    pub fn new(radio: Peri<'d, RADIO>) -> Self {
        Self(ieee802154::Radio::new(radio, crate::irqs::Irqs))
    }

    /// Changes the radio channel.
    ///
    /// # Panics
    ///
    /// Outside channels 11 to 26.
    pub fn set_channel(&mut self, channel: u8) {
        self.0.set_channel(channel);
    }

    /// Changes the Clear Channel Assessment method.
    pub fn set_cca(&mut self, cca: Cca) {
        self.0.set_cca(cca);
    }

    /// Changes the Start of Frame Delimiter (SFD).
    pub fn set_sfd(&mut self, sfd: u8) {
        self.0.set_sfd(sfd);
    }

    /// Clears interrupts.
    pub fn clear_all_interrupts(&mut self) {
        self.0.clear_all_interrupts();
    }

    /// Changes the radio transmission power, in dBm.
    ///
    /// # Panics
    ///
    /// For a power the chip doesn't have.
    pub fn set_transmission_power(&mut self, power: i8) {
        self.0.set_transmission_power(power);
    }

    /// Receives one radio packet into `packet`.
    ///
    /// # Errors
    ///
    /// [`Error::CrcFailed`] with the CRC as received.
    pub async fn receive(&mut self, packet: &mut Packet) -> Result<(), Error> {
        self.0.receive(packet).await
    }

    /// Runs CCA, then sends `packet` if the channel is clear.
    ///
    /// # Errors
    ///
    /// [`Error::ChannelInUse`] if CCA found the channel busy.
    pub async fn try_send(&mut self, packet: &mut Packet) -> Result<(), Error> {
        self.0.try_send(packet).await
    }
}
