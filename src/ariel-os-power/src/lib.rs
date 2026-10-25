//! Provides power management functionality.

#![deny(missing_docs)]
#![cfg_attr(not(context = "native-chip"), no_std)]

mod reset;

pub use reset::*;
