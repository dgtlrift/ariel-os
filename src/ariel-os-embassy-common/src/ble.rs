//! Common BLE types to be used across different HALs.

use static_cell::StaticCell;
use trouble_host::{HostResources, prelude::DefaultPacketPool};

/// Maximum Transmission Unit (MTU) for BLE connections. 27 should work for all BLE versions.
///
/// Since bluetooth 4.2, this can be increased to 251 bytes.
/// Set it at build time with `CONFIG_BLE_MTU`.
pub const MTU: usize = ariel_os_utils::usize_from_env_or!(
    "CONFIG_BLE_MTU",
    27,
    "maximum transmission unit (MTU) of BLE connections"
);

/// Maximum number of concurrent connections to be handled by the BLE stack, minimum 1.
/// Set it at build time with `CONFIG_BLE_MAX_CONNS`.
pub const MAX_CONNS: usize = ariel_os_utils::usize_from_env_or!(
    "CONFIG_BLE_MAX_CONNS",
    1,
    "maximum number of concurrent BLE connections"
);
/// Maximum number of concurrent channels to be handled by the BLE stack (not including GATT), minimum 1.
/// Set it at build time with `CONFIG_BLE_MAX_CHANNELS`.
pub const MAX_CHANNELS: usize = ariel_os_utils::usize_from_env_or!(
    "CONFIG_BLE_MAX_CHANNELS",
    1,
    "maximum number of concurrent L2CAP channels, not counting GATT"
);

const _: () = assert!(MAX_CONNS >= 1 && MAX_CHANNELS >= 1, "BLE limits must be at least 1");
const _: () = assert!(MTU >= 27 && MTU <= 251, "CONFIG_BLE_MTU must be 27 to 251");

static HOST_RESOURCES: StaticCell<BleHostResources> = StaticCell::new();

/// Alias for the BLE host resources, setting the generic parameters to the constants for convenience.
pub type BleHostResources = HostResources<DefaultPacketPool, MAX_CONNS, MAX_CHANNELS, MTU>;

/// Initializes the BLE host resources.
///
/// Call this function to get the `HostResources` instance that can be used to initialize the trouBLE stack.
pub fn get_ble_host_resources() -> &'static mut BleHostResources {
    HOST_RESOURCES.init(HostResources::new())
}

/// Configuration for the BLE stack.
pub struct Config {
    /// The address of the BLE device.
    pub address: trouble_host::Address,
}

/// Default address, this one is used in the trouBLE examples, needs to be changed/randomized in production.
impl Default for Config {
    fn default() -> Self {
        Self {
            address: trouble_host::Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]),
        }
    }
}
