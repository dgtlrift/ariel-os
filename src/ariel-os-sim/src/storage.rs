//! The program flash, for Ariel `storage`: `flash` requests to the kernel, which keeps the
//! node's `program-flash.bin`. The node is stalled for a write or erase, as an nRF52840 is
//! while its NVMC works, so each call returns when the operation ends.

use crate::wire::generated::FlashMsg;
use crate::wire::{Flash as FlashRequest, FlashOp, NodeMessage};
use embedded_storage_async::nor_flash::{
    ErrorType, MultiwriteNorFlash, NorFlash, NorFlashError, NorFlashErrorKind, ReadNorFlash,
};

use crate::kernel;

/// The flash's size; the kernel's program flash must match (the sim board's sidecar).
const CAPACITY: usize = 1024 * 1024;

/// The program flash.
pub struct Flash {
    _private: (),
}

/// A flash operation the firmware got wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlashError(NorFlashErrorKind);

impl NorFlashError for FlashError {
    fn kind(&self) -> NorFlashErrorKind {
        self.0
    }
}

impl From<NorFlashErrorKind> for FlashError {
    fn from(kind: NorFlashErrorKind) -> Self {
        Self(kind)
    }
}

impl ErrorType for Flash {
    type Error = FlashError;
}

fn request(
    op: FlashOp,
    offset: u32,
    data: Option<&[u8]>,
    len: Option<usize>,
    to: Option<u32>,
) -> Vec<u8> {
    kernel::with(|k| {
        let erase = matches!(op, FlashOp::Erase);
        let reply = k.request(&NodeMessage::Flash(FlashRequest {
            msg: FlashMsg,
            t: k.now(),
            op,
            offset: (!erase).then_some(u64::from(offset)),
            len: len.map(|n| n as u64),
            data: data.map(<[u8]>::to_vec),
            from: erase.then_some(u64::from(offset)),
            to: to.map(u64::from),
        }));
        reply.data.unwrap_or_default()
    })
}

/// `embedded-storage`'s checks: in bounds, and aligned to `unit`.
///
/// # Errors
///
/// `OutOfBounds` past the flash's end, `NotAligned` otherwise.
fn check(offset: u32, len: usize, unit: usize) -> Result<(), FlashError> {
    let end = offset as usize + len;
    if end > CAPACITY {
        return Err(NorFlashErrorKind::OutOfBounds.into());
    }
    if !(offset as usize).is_multiple_of(unit) || !len.is_multiple_of(unit) {
        return Err(NorFlashErrorKind::NotAligned.into());
    }
    Ok(())
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the kernel answers synchronously (lockstep)"
)]
impl ReadNorFlash for Flash {
    const READ_SIZE: usize = 1;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        check(offset, bytes.len(), Self::READ_SIZE)?;
        for (i, chunk) in bytes.chunks_mut(crate::wire::MAX_CHUNK).enumerate() {
            let at = offset + u32::try_from(i * crate::wire::MAX_CHUNK).unwrap_or(u32::MAX);
            let data = request(FlashOp::Read, at, None, Some(chunk.len()), None);
            for (b, d) in chunk.iter_mut().zip(data) {
                *b = d;
            }
        }
        Ok(())
    }

    fn capacity(&self) -> usize {
        CAPACITY
    }
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "the kernel answers synchronously (lockstep)"
)]
impl NorFlash for Flash {
    const WRITE_SIZE: usize = 4;
    const ERASE_SIZE: usize = 4096;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        if from > to {
            return Err(NorFlashErrorKind::OutOfBounds.into());
        }
        check(from, (to - from) as usize, Self::ERASE_SIZE)?;
        request(FlashOp::Erase, from, None, None, Some(to));
        Ok(())
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        check(offset, bytes.len(), Self::WRITE_SIZE)?;
        for (i, chunk) in bytes.chunks(crate::wire::MAX_CHUNK).enumerate() {
            let at = offset + u32::try_from(i * crate::wire::MAX_CHUNK).unwrap_or(u32::MAX);
            request(FlashOp::Write, at, Some(chunk), None, None);
        }
        Ok(())
    }
}

// Bits only ever clear, so writing a word again is allowed, as on the nRF52840.
impl MultiwriteNorFlash for Flash {}

#[doc(hidden)]
pub fn init(peripherals: &mut crate::OptionalPeripherals) -> Flash {
    let _ = peripherals.FLASH.take().expect("FLASH has not been taken");
    Flash { _private: () }
}

/// Ariel `storage`'s region of the program flash, from the simulator's board.
#[must_use]
pub fn flash_range() -> core::ops::Range<u32> {
    kernel::with(|k| k.identity.storage.clone())
}
