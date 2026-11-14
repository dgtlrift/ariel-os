//! The chip's own 802.15.4 radio: the nRF52840's `RADIO` in its IEEE 802.15.4 mode
//! (ariel-sim D60, D63).
//!
//! The API is embassy-nrf's `radio::ieee802154`, method for method, so an app written
//! against `ariel_os::hal::ieee802154` builds for the `sim` board and for a real nRF52840;
//! only [`Radio::new`] takes no interrupt binding. Each operation is a `radio` message to
//! the simulator's model of the chip, whose end comes back with the kernel's `advance` or
//! `reply`. The radio is shared with the BLE controller, as on the chip: an operation can
//! wait for it.
//!
//! The frame is the firmware's own octets: the simulator neither parses nor checks it,
//! beyond the FCS the radio adds and checks, as the chip does.

use core::marker::PhantomData;
use core::task::Poll;

use embassy_hal_internal::Peri;
use embassy_hal_internal::drop::OnDrop;

use crate::kernel;
use crate::peripherals::RADIO;
use crate::wire::generated::RadioMsg;
use crate::wire::{NodeMessage, Radio as RadioRequest, RadioCca, RadioEventKind, RadioOp};

/// Default (IEEE compliant) Start of Frame Delimiter.
pub const DEFAULT_SFD: u8 = 0xA7;

/// RADIO error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// Buffer was too long.
    BufferTooLong,
    /// Buffer was too short.
    BufferTooShort,
    /// The buffer is not in data RAM (never, in the simulator).
    BufferNotInRAM,
    /// Clear channel assessment reported channel in use.
    ChannelInUse,
    /// CRC check failed.
    CrcFailed(u16),
}

/// Clear Channel Assessment method.
pub enum Cca {
    /// Carrier sense.
    CarrierSense,
    /// Energy Detection / Energy Above Threshold.
    EnergyDetection {
        /// Energy measurements above this value mean that the channel is assumed to be
        /// busy: dB above the chip's `ED_RSSIOFFS` (nRF52840 Product Specification
        /// section 6.20.12.4).
        ed_threshold: u8,
    },
}

/// The settings, as the model takes them.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Settings {
    channel: u8,
    power: i8,
    cca: Option<u8>,
    sfd: u8,
}

/// IEEE 802.15.4 radio driver.
pub struct Radio<'d> {
    settings: Settings,
    /// A setting changed since the model last had them.
    dirty: bool,
    phantom: PhantomData<&'d ()>,
}

impl<'d> Radio<'d> {
    /// Creates a new IEEE 802.15.4 radio driver: channel 11, 0 dBm, carrier-sense CCA, the
    /// standard SFD.
    #[must_use]
    pub fn new(_radio: Peri<'d, RADIO>) -> Self {
        Self {
            settings: Settings {
                channel: 11,
                power: 0,
                cca: None,
                sfd: DEFAULT_SFD,
            },
            dirty: true,
            phantom: PhantomData,
        }
    }

    /// Changes the radio channel.
    ///
    /// # Panics
    ///
    /// Outside channels 11 to 26.
    pub fn set_channel(&mut self, channel: u8) {
        assert!((11..=26).contains(&channel), "Bad 802.15.4 channel");
        self.settings.channel = channel;
        self.dirty = true;
    }

    /// Changes the Clear Channel Assessment method.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "embassy-nrf's API, which apps for the real chip use"
    )]
    pub fn set_cca(&mut self, cca: Cca) {
        self.settings.cca = match cca {
            Cca::CarrierSense => None,
            Cca::EnergyDetection { ed_threshold } => Some(ed_threshold),
        };
        self.dirty = true;
    }

    /// Changes the Start of Frame Delimiter (SFD).
    pub fn set_sfd(&mut self, sfd: u8) {
        self.settings.sfd = sfd;
        self.dirty = true;
    }

    /// Clears interrupts (nothing to clear in the simulator).
    pub fn clear_all_interrupts(&mut self) {}

    /// Changes the radio transmission power, in dBm.
    ///
    /// # Panics
    ///
    /// For a power the nRF52840 doesn't have.
    pub fn set_transmission_power(&mut self, power: i8) {
        assert!(
            matches!(
                power,
                8 | 7 | 6 | 5 | 4 | 3 | 2 | 0 | -4 | -8 | -12 | -16 | -20 | -30 | -40
            ),
            "Invalid transmission power value"
        );
        self.settings.power = power;
        self.dirty = true;
    }

    fn request(op: RadioOp, settings: Option<Settings>, psdu: Option<&[u8]>) {
        kernel::with(|k| {
            let msg = NodeMessage::Radio(RadioRequest {
                msg: RadioMsg,
                t: k.now(),
                op,
                channel: settings.map(|s| u64::from(s.channel)),
                power_dbm: settings.map(|s| i64::from(s.power)),
                cca: settings.map(|s| match s.cca {
                    Some(_) => RadioCca::Energy,
                    None => RadioCca::Carrier,
                }),
                ed_threshold: settings.and_then(|s| s.cca).map(u64::from),
                sfd: settings.map(|s| u64::from(s.sfd)),
                psdu: psdu.map(<[u8]>::to_vec),
            });
            k.request(&msg);
        });
    }

    /// Gives the model the settings if they changed, and forgets the ends of operations
    /// that were stopped.
    fn prepare(&mut self) {
        kernel::with(kernel::Kernel::radio_clear);
        if self.dirty {
            Self::request(RadioOp::Configure, Some(self.settings), None);
            self.dirty = false;
        }
    }

    /// Waits for the operation's end.
    async fn end() -> crate::wire::RadioEvent {
        core::future::poll_fn(|cx| {
            kernel::with(|k| {
                if let Some(e) = k.radio_take() {
                    Poll::Ready(e)
                } else {
                    k.wake_on_radio(cx.waker());
                    Poll::Pending
                }
            })
        })
        .await
    }

    /// Receives one radio packet and copies its contents into the given `packet` buffer.
    ///
    /// This method returns the `Ok` variant if the CRC included in the packet was
    /// successfully validated by the hardware; otherwise it returns the `Err` variant. In
    /// either case, `packet` will be updated with the received packet's data.
    ///
    /// # Errors
    ///
    /// [`Error::CrcFailed`] with the CRC as received.
    ///
    /// # Panics
    ///
    /// If the simulator ends the operation as other than a reception: its bug.
    pub async fn receive(&mut self, packet: &mut Packet) -> Result<(), Error> {
        self.prepare();
        Self::request(RadioOp::Receive, None, None);
        let dropper = OnDrop::new(|| Self::request(RadioOp::Stop, None, None));
        let e = Self::end().await;
        dropper.defuse();
        assert_eq!(e.kind, RadioEventKind::Received, "ariel-sim: radio event");
        let psdu = e.psdu.unwrap_or_default();
        let n = psdu.len().min(usize::from(Packet::CAPACITY));
        packet.copy_from_slice(psdu.get(..n).unwrap_or_default());
        if let Some(lqi) = packet.buffer.get_mut(1 + n) {
            *lqi = u8::try_from(e.lqi.unwrap_or(0).min(255)).unwrap_or(u8::MAX);
        }
        if e.fcs_ok == Some(true) {
            Ok(())
        } else {
            Err(Error::CrcFailed(
                u16::try_from(e.fcs.unwrap_or(0)).unwrap_or(0),
            ))
        }
    }

    /// Tries to send the given `packet`.
    ///
    /// This method performs Clear Channel Assessment (CCA) first and sends the `packet`
    /// only if the channel is observed to be *clear* (no transmission is currently
    /// ongoing), otherwise no packet is transmitted and the `Err` variant is returned.
    ///
    /// NOTE this method will *not* modify the `packet` argument; the mutable reference is
    /// embassy-nrf's, which needs it in RAM.
    ///
    /// # Errors
    ///
    /// [`Error::ChannelInUse`] if CCA found the channel busy.
    ///
    /// # Panics
    ///
    /// If the simulator ends the operation as other than a send: its bug.
    pub async fn try_send(&mut self, packet: &mut Packet) -> Result<(), Error> {
        self.prepare();
        Self::request(RadioOp::Send, None, Some(packet));
        let dropper = OnDrop::new(|| Self::request(RadioOp::Stop, None, None));
        let e = Self::end().await;
        dropper.defuse();
        match e.kind {
            RadioEventKind::Sent => Ok(()),
            RadioEventKind::Busy => Err(Error::ChannelInUse),
            RadioEventKind::Received => panic!("ariel-sim: a send ended as a reception"),
        }
    }

    /// Sends the given `packet` at once, without CCA: for an 802.15.4 ACK, which goes
    /// `aTurnaroundTime` after its frame whatever the channel (802.15.4-2020 section
    /// 6.7.4.3). Not in embassy-nrf's API (ariel-sim D62-1c).
    ///
    /// From RXIDLE, where [`Radio::receive`] leaves the radio, the frame starts after
    /// the RX-to-TX turnaround (40 µs, nRF52840 Product Specification section
    /// 6.20.15.8); otherwise after `tTXEN` (130 µs).
    ///
    /// # Errors
    ///
    /// None in the simulator; the `Result` matches [`Radio::try_send`]'s.
    ///
    /// # Panics
    ///
    /// If the simulator ends the operation as other than a send: its bug.
    pub async fn send_now(&mut self, packet: &mut Packet) -> Result<(), Error> {
        self.prepare();
        Self::request(RadioOp::SendNow, None, Some(packet));
        let dropper = OnDrop::new(|| Self::request(RadioOp::Stop, None, None));
        let e = Self::end().await;
        dropper.defuse();
        assert_eq!(e.kind, RadioEventKind::Sent, "ariel-sim: radio event");
        Ok(())
    }
}

/// An IEEE 802.15.4 packet.
///
/// This `Packet` is a PHY layer packet. It's made up of the physical header (PHR) and the
/// PSDU (PHY service data unit). The PSDU of this `Packet` will always include the MAC
/// level CRC, AKA the FCS (Frame Control Sequence) -- the CRC is fully computed in
/// hardware and automatically appended on transmission and verified on reception.
///
/// The API lets users modify the usable part (not the CRC) of the PSDU via the `deref` and
/// `copy_from_slice` methods. These methods will automatically update the PHR.
pub struct Packet {
    buffer: [u8; Self::SIZE],
}

impl Packet {
    const PHY_HDR: usize = 0;
    const DATA: core::ops::RangeFrom<usize> = 1..;

    /// Maximum amount of usable payload (CRC excluded) a single packet can contain, in
    /// bytes.
    pub const CAPACITY: u8 = 125;
    const CRC: u8 = 2;
    const MAX_PSDU_LEN: u8 = Self::CAPACITY + Self::CRC;
    const SIZE: usize = 1 + Self::MAX_PSDU_LEN as usize;

    /// Returns an empty packet (length = 0).
    #[must_use]
    #[expect(
        clippy::new_without_default,
        reason = "embassy-nrf's API, which apps for the real chip use"
    )]
    pub fn new() -> Self {
        let mut packet = Self {
            buffer: [0; Self::SIZE],
        };
        packet.set_len(0);
        packet
    }

    /// Fills the packet payload with given `src` data.
    ///
    /// # Panics
    ///
    /// This function panics if `src` is larger than `Self::CAPACITY`.
    pub fn copy_from_slice(&mut self, src: &[u8]) {
        assert!(src.len() <= Self::CAPACITY as usize);
        let len = u8::try_from(src.len()).expect("checked above");
        for (d, s) in self.buffer.iter_mut().skip(Self::DATA.start).zip(src) {
            *d = *s;
        }
        self.set_len(len);
    }

    /// Returns the size of this packet's payload.
    #[must_use]
    #[expect(
        clippy::len_without_is_empty,
        reason = "embassy-nrf's API, which apps for the real chip use"
    )]
    pub fn len(&self) -> u8 {
        self.buffer[Self::PHY_HDR] - Self::CRC
    }

    /// Changes the size of the packet's payload.
    ///
    /// # Panics
    ///
    /// This function panics if `len` is larger than `Self::CAPACITY`.
    pub fn set_len(&mut self, len: u8) {
        assert!(len <= Self::CAPACITY);
        self.buffer[Self::PHY_HDR] = len + Self::CRC;
    }

    /// Returns the LQI (Link Quality Indicator) of the received packet.
    ///
    /// The value is only valid after a `Radio::receive`; operations that modify the
    /// `Packet` overwrite it.
    #[must_use]
    pub fn lqi(&self) -> u8 {
        self.buffer
            .get(Self::DATA.start + usize::from(self.len()))
            .copied()
            .unwrap_or(0)
    }
}

impl core::ops::Deref for Packet {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        let end = Self::DATA.start + usize::from(self.len());
        self.buffer.get(Self::DATA.start..end).unwrap_or_default()
    }
}

impl core::ops::DerefMut for Packet {
    fn deref_mut(&mut self) -> &mut [u8] {
        let end = Self::DATA.start + usize::from(self.len());
        self.buffer
            .get_mut(Self::DATA.start..end)
            .unwrap_or_default()
    }
}
