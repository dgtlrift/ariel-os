//! The node's entropy: a generator seeded from the simulator's seed for this node (D12),
//! never the host's, so a run replays.

use rand_core::SeedableRng as _;

/// Seeds Ariel's RNG.
pub fn construct_rng(_peripherals: &mut crate::OptionalPeripherals) {
    let seed = crate::kernel::with(|k| k.identity.rng_seed);
    let mut rng = rand_chacha::ChaCha20Rng::seed_from_u64(seed);
    ariel_os_random::construct_rng(&mut rng);
}
