//! Provides GPIO access: pins as the simulator's board wires them.
//!
//! An input's level is the last one the kernel gave, with every `advance` and `reply`. An
//! output sends a `pin-set` to the kernel each time its level changes; chip selects and
//! device reset lines are outputs.

use core::marker::PhantomData;

use crate::wire::generated::{PinConfigMsg, PinSetMsg};
use crate::wire::{NodeMessage, PinConfig, PinConfigDir, PinConfigPull, PinSet};
use embassy_hal_internal::{Peri, PeripheralType};

use crate::kernel;

/// A board pin, by the name the simulator's board uses.
pub trait Pin: PeripheralType + 'static {
    /// The pin's name (`P0_13`).
    const NAME: &'static str;
}

macro_rules! impl_pins {
    ($($name:ident),* $(,)?) => {
        $(
            impl Pin for crate::peripherals::$name {
                const NAME: &'static str = stringify!($name);
            }
        )*
    };
}

impl_pins!(
    P0_00, P0_01, P0_02, P0_03, P0_04, P0_05, P0_06, P0_07, P0_08, P0_09, P0_10, P0_11, P0_12,
    P0_13, P0_14, P0_15, P0_16, P0_17, P0_18, P0_19, P0_20, P0_21, P0_22, P0_23, P0_24, P0_25,
    P0_26, P0_27, P0_28, P0_29, P0_30, P0_31,
);
// The nRF52840's port 1 (D40).
#[cfg(context = "nrf52840")]
impl_pins!(
    P1_00, P1_01, P1_02, P1_03, P1_04, P1_05, P1_06, P1_07, P1_08, P1_09, P1_10, P1_11, P1_12,
    P1_13, P1_14, P1_15,
);

/// Tells the kernel a pin's direction and returns the level the kernel reports for it.
fn configure(
    pin: &'static str,
    dir: PinConfigDir,
    level: Option<bool>,
    pull: Option<PinConfigPull>,
) -> bool {
    kernel::with(|k| {
        let reply = k.request(&NodeMessage::PinConfig(PinConfig {
            msg: PinConfigMsg,
            t: k.now(),
            pin: pin.into(),
            dir,
            level,
            pull,
        }));
        reply.level.unwrap_or(false)
    })
}

pub mod input {
    //! Input-specific types.

    use core::future::poll_fn;
    use core::task::Poll;

    use ariel_os_embassy_common::gpio::Pull;

    use super::{Peri, PhantomData, PinConfigDir, PinConfigPull, configure, kernel};

    #[doc(hidden)]
    pub use super::Pin as InputPin;

    /// Whether inputs support configuring whether a Schmitt trigger is enabled.
    pub const SCHMITT_TRIGGER_CONFIGURABLE: bool = false;

    /// A logic level.
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Level {
        /// Low.
        Low,
        /// High.
        High,
    }

    /// An input pin. Every input can wait for edges, as on nRF.
    #[doc(hidden)]
    pub struct Input<'d> {
        pin: &'static str,
        _marker: PhantomData<&'d mut ()>,
    }

    #[cfg(feature = "external-interrupts")]
    #[doc(hidden)]
    pub use Input as IntEnabledInput;

    #[doc(hidden)]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the HAL interface: the pin is a token"
    )]
    pub fn new<P: InputPin>(
        pin: Peri<'_, P>,
        pull: Pull,
        _schmitt_trigger: bool,
    ) -> Result<Input<'_>, ariel_os_embassy_common::gpio::input::Error> {
        let _ = pin;
        let pull = match pull {
            Pull::None => None,
            Pull::Up => Some(PinConfigPull::Up),
            Pull::Down => Some(PinConfigPull::Down),
        };
        let level = configure(P::NAME, PinConfigDir::In, None, pull);
        kernel::with(|k| k.track_input(P::NAME, level));
        Ok(Input {
            pin: P::NAME,
            _marker: PhantomData,
        })
    }

    #[cfg(feature = "external-interrupts")]
    #[doc(hidden)]
    pub fn new_int_enabled<P: InputPin>(
        pin: Peri<'_, P>,
        pull: Pull,
        schmitt_trigger: bool,
    ) -> Result<IntEnabledInput<'_>, ariel_os_embassy_common::gpio::input::Error> {
        new(pin, pull, schmitt_trigger)
    }

    impl Input<'_> {
        /// Whether the input is high.
        #[must_use]
        pub fn is_high(&self) -> bool {
            kernel::with(|k| k.level(self.pin))
        }

        /// Whether the input is low.
        #[must_use]
        pub fn is_low(&self) -> bool {
            !self.is_high()
        }

        /// The input's level.
        #[must_use]
        pub fn get_level(&self) -> Level {
            if self.is_high() {
                Level::High
            } else {
                Level::Low
            }
        }

        /// Waits until the input is high.
        pub async fn wait_for_high(&mut self) {
            self.wait_until(kernel::Kernel::level).await;
        }

        /// Waits until the input is low.
        pub async fn wait_for_low(&mut self) {
            self.wait_until(|k, pin| !k.level(pin)).await;
        }

        /// Waits for the next rising edge.
        pub async fn wait_for_rising_edge(&mut self) {
            let (rising, _) = kernel::with(|k| k.edges(self.pin));
            self.wait_until(move |k, pin| k.edges(pin).0 != rising)
                .await;
        }

        /// Waits for the next falling edge.
        pub async fn wait_for_falling_edge(&mut self) {
            let (_, falling) = kernel::with(|k| k.edges(self.pin));
            self.wait_until(move |k, pin| k.edges(pin).1 != falling)
                .await;
        }

        /// Waits for the next edge.
        pub async fn wait_for_any_edge(&mut self) {
            let before = kernel::with(|k| k.edges(self.pin));
            self.wait_until(move |k, pin| k.edges(pin) != before).await;
        }

        async fn wait_until(&self, done: impl Fn(&kernel::Kernel, &'static str) -> bool) {
            let pin = self.pin;
            poll_fn(|cx| {
                kernel::with(|k| {
                    if done(k, pin) {
                        Poll::Ready(())
                    } else {
                        k.wake_on_edge(pin, cx.waker());
                        Poll::Pending
                    }
                })
            })
            .await;
        }
    }

    impl embedded_hal::digital::ErrorType for Input<'_> {
        type Error = core::convert::Infallible;
    }

    impl embedded_hal::digital::InputPin for Input<'_> {
        fn is_high(&mut self) -> Result<bool, Self::Error> {
            Ok(Input::is_high(self))
        }

        fn is_low(&mut self) -> Result<bool, Self::Error> {
            Ok(Input::is_low(self))
        }
    }

    impl embedded_hal_async::digital::Wait for Input<'_> {
        async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
            Input::wait_for_high(self).await;
            Ok(())
        }

        async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
            Input::wait_for_low(self).await;
            Ok(())
        }

        async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
            Input::wait_for_rising_edge(self).await;
            Ok(())
        }

        async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
            Input::wait_for_falling_edge(self).await;
            Ok(())
        }

        async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
            Input::wait_for_any_edge(self).await;
            Ok(())
        }
    }

    ariel_os_embassy_common::define_into_level!();
}

pub mod output {
    //! Output-specific types.

    use super::{
        DriveStrength, NodeMessage, Peri, PhantomData, PinConfigDir, PinSet, PinSetMsg, Speed,
        configure, kernel,
    };

    #[doc(hidden)]
    pub use super::Pin as OutputPin;

    /// Whether outputs support configuring their drive strength.
    pub const DRIVE_STRENGTH_CONFIGURABLE: bool = false;
    /// Whether outputs support configuring their speed/slew rate.
    pub const SPEED_CONFIGURABLE: bool = false;

    /// An output pin.
    #[doc(hidden)]
    pub struct Output<'d> {
        pin: &'static str,
        level: bool,
        _marker: PhantomData<&'d mut ()>,
    }

    #[doc(hidden)]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the HAL interface: the pin is a token"
    )]
    pub fn new<P: OutputPin>(
        pin: Peri<'_, P>,
        initial_level: ariel_os_embassy_common::gpio::Level,
        _drive_strength: DriveStrength,
        _speed: Speed,
    ) -> Output<'_> {
        let _ = pin;
        let level = matches!(initial_level, ariel_os_embassy_common::gpio::Level::High);
        configure(P::NAME, PinConfigDir::Out, Some(level), None);
        Output {
            pin: P::NAME,
            level,
            _marker: PhantomData,
        }
    }

    impl Output<'_> {
        fn set(&mut self, level: bool) {
            if level == self.level {
                return;
            }
            self.level = level;
            let pin = self.pin;
            kernel::with(|k| {
                k.request(&NodeMessage::PinSet(PinSet {
                    msg: PinSetMsg,
                    t: k.now(),
                    pin: pin.into(),
                    level,
                }));
            });
        }
    }

    impl embedded_hal::digital::ErrorType for Output<'_> {
        type Error = core::convert::Infallible;
    }

    impl embedded_hal::digital::OutputPin for Output<'_> {
        fn set_low(&mut self) -> Result<(), Self::Error> {
            self.set(false);
            Ok(())
        }

        fn set_high(&mut self) -> Result<(), Self::Error> {
            self.set(true);
            Ok(())
        }
    }

    impl embedded_hal::digital::StatefulOutputPin for Output<'_> {
        fn is_set_high(&mut self) -> Result<bool, Self::Error> {
            Ok(self.level)
        }

        fn is_set_low(&mut self) -> Result<bool, Self::Error> {
            Ok(!self.level)
        }
    }
}

pub use ariel_os_embassy_common::gpio::UnsupportedDriveStrength as DriveStrength;
pub use ariel_os_embassy_common::gpio::UnsupportedSpeed as Speed;
