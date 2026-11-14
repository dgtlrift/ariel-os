//! The chip's IEEE 802.15.4 radio: embassy-nrf's `radio::ieee802154` driver, with its
//! interrupt bound here (ariel-sim D60).
//!
//! The same API is the sim HAL's `ieee802154`, so an app written against
//! `ariel_os::hal::ieee802154` builds for an nRF52840 and for the ariel-sim simulator. On
//! the chip, BLE and this driver can't share the radio yet: that needs MPSL's timeslot
//! API, which the simulator models and Ariel doesn't use.

#[cfg(feature = "ble")]
compile_error!("ieee802154 and ble both need the nRF52840's RADIO, and Ariel can't share it yet");

use core::{
    future::poll_fn,
    sync::atomic::{Ordering, compiler_fence},
    task::Poll,
};

use embassy_nrf::{
    Peri,
    pac::{self, radio::vals::State},
    peripherals::RADIO,
    radio::ieee802154,
};

pub use embassy_nrf::radio::Error;
pub use embassy_nrf::radio::ieee802154::{Cca, DEFAULT_SFD, Packet};

/// IEEE 802.15.4 radio driver: embassy-nrf's, its interrupt bound by the HAL.
pub struct Radio<'d> {
    radio: ieee802154::Radio<'d>,
    /// A setting changed since embassy-nrf last enabled the radio, so [`Radio::send_now`]
    /// can't turn around from RXIDLE.
    retuned: bool,
}

impl<'d> Radio<'d> {
    /// Creates a new IEEE 802.15.4 radio driver: channel 11, 0 dBm, carrier-sense CCA, the
    /// standard SFD.
    #[must_use]
    pub fn new(radio: Peri<'d, RADIO>) -> Self {
        Self {
            radio: ieee802154::Radio::new(radio, crate::irqs::Irqs),
            retuned: true,
        }
    }

    /// Changes the radio channel.
    ///
    /// # Panics
    ///
    /// Outside channels 11 to 26.
    pub fn set_channel(&mut self, channel: u8) {
        self.retuned = true;
        self.radio.set_channel(channel);
    }

    /// Changes the Clear Channel Assessment method.
    pub fn set_cca(&mut self, cca: Cca) {
        self.retuned = true;
        self.radio.set_cca(cca);
    }

    /// Changes the Start of Frame Delimiter (SFD).
    pub fn set_sfd(&mut self, sfd: u8) {
        self.retuned = true;
        self.radio.set_sfd(sfd);
    }

    /// Clears interrupts.
    pub fn clear_all_interrupts(&mut self) {
        self.radio.clear_all_interrupts();
    }

    /// Changes the radio transmission power, in dBm.
    ///
    /// # Panics
    ///
    /// For a power the chip doesn't have.
    pub fn set_transmission_power(&mut self, power: i8) {
        self.retuned = true;
        self.radio.set_transmission_power(power);
    }

    /// Receives one radio packet into `packet`.
    ///
    /// # Errors
    ///
    /// [`Error::CrcFailed`] with the CRC as received.
    pub async fn receive(&mut self, packet: &mut Packet) -> Result<(), Error> {
        self.retuned = false;
        self.radio.receive(packet).await
    }

    /// Runs CCA, then sends `packet` if the channel is clear.
    ///
    /// # Errors
    ///
    /// [`Error::ChannelInUse`] if CCA found the channel busy.
    pub async fn try_send(&mut self, packet: &mut Packet) -> Result<(), Error> {
        self.retuned = false;
        self.radio.try_send(packet).await
    }

    /// Sends `packet` at once, without CCA: for an 802.15.4 ACK, which goes
    /// `aTurnaroundTime` after its frame whatever the channel (802.15.4-2020 section
    /// 6.7.4.3). Not in embassy-nrf's API (ariel-sim D62-1c).
    ///
    /// From RXIDLE, where [`Radio::receive`] leaves the radio, the frame starts after the
    /// RX-to-TX turnaround (at most 40 µs, nRF52840 Product Specification section
    /// 6.20.15.8); otherwise the radio is disabled first and starts after `tTXEN`
    /// (130 µs). It waits for the frame's end by polling, without the interrupt, which
    /// embassy-nrf's driver owns. The radio is left disabled, as `try_send` leaves it.
    ///
    /// # Errors
    ///
    /// None; the `Result` matches [`Radio::try_send`]'s.
    pub async fn send_now(&mut self, packet: &mut Packet) -> Result<(), Error> {
        let r = pac::RADIO;
        // The PHR, then the PSDU (at most 127 octets) without its FCS, which the radio
        // adds.
        let mut buf = [0u8; 128];
        let len = packet.len();
        if let Some((phr, psdu)) = buf.split_first_mut() {
            *phr = len + 2;
            for (d, s) in psdu.iter_mut().zip(packet.iter()) {
                *d = *s;
            }
        }
        if self.retuned || r.state().read().state() != State::RxIdle {
            r.shorts().write(|_| {});
            r.events_disabled().write_value(0);
            r.tasks_disable().write_value(1);
            while r.events_disabled().read() == 0 {}
            r.events_disabled().write_value(0);
        }
        self.retuned = false;
        r.events_phyend().write_value(0);
        r.shorts().write(|w| {
            w.set_txready_start(true);
            w.set_phyend_disable(true);
        });
        r.packetptr().write_value(buf.as_ptr() as u32);
        // The DMA reads `buf` after this.
        compiler_fence(Ordering::SeqCst);
        r.tasks_txen().write_value(1);
        // A dropped future stops the frame.
        let stop = StopOnDrop;
        poll_fn(|cx| {
            if r.events_phyend().read() == 0 {
                cx.waker().wake_by_ref();
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
        core::mem::forget(stop);
        r.events_phyend().write_value(0);
        compiler_fence(Ordering::SeqCst);
        Ok(())
    }
}

/// Disables the radio when dropped: a [`Radio::send_now`] future dropped mid-frame.
struct StopOnDrop;

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        let r = pac::RADIO;
        r.shorts().write(|_| {});
        r.tasks_disable().write_value(1);
    }
}
