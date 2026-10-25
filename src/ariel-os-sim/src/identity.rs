/// The node's identity: the EUI-64 the simulator gave it (D13).
pub struct DeviceId([u8; 8]);

impl ariel_os_embassy_common::identity::DeviceId for DeviceId {
    #[expect(
        refining_impl_trait_reachable,
        reason = "as on nRF: the identity is always available"
    )]
    fn get() -> Result<Self, core::convert::Infallible> {
        Ok(Self(crate::kernel::with(|k| k.identity.eui64)))
    }

    type Bytes = [u8; 8];

    fn bytes(&self) -> Self::Bytes {
        self.0
    }
}
