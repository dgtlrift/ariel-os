# ariel-os-sim

The Ariel OS HAL for `sim-chip`: firmware as a host process inside the ariel-sim simulator (plan section 6.10 of ariel-sim). It plays the role `ariel-os-nrf` plays for nRF chips; `ariel-os-hal` dispatches to it under `cfg(context = "sim-chip")`.

A firmware binary built for `-b sim` is meant to be started by the simulator (`sim-node`), not by hand. Started by hand, without `ARIEL_SIM_SOCKET`, it exits at once with a message saying so.

## Layout

It has the module layout every Ariel HAL has, with `ariel-os-hal`'s dummy HAL (`src/ariel-os-hal/src/hal/dummy`) as the checklist of items:

| Module | What it provides | Talks to the kernel with |
| --- | --- | --- |
| `peripherals` | `P0_00` to `P0_31`, `SPI0`, `I2C0`, `FLASH`: singletons from `embassy-hal-internal`'s `peripherals_definition!`, as other HALs define them, and an `OptionalPeripherals` holding one of each | – |
| `init()` | connects to `ARIEL_SIM_SOCKET`, sends `hello`, takes `start` (identity, seed, storage range, boot time), returns `OptionalPeripherals` | `hello`, `start` |
| `executor` | `Executor` with `run(init) -> !`: the lockstep loop. With the peripherals' one-time construction, the only `unsafe` (`__pender`, `raw::Executor::poll`) | `idle`, `advance` |
| `time` | `embassy-time-driver` over the node's virtual clock, ticks at 1 MHz (D8): `now = floor(t_ns / 1000)`, an alarm at tick `k` is due at `k × 1000` ns. Timers are kept with `embassy-time-queue-utils` | – (`idle.next_wake`) |
| `gpio` | `Input`, `IntEnabledInput` (with `wait_for_high`, `wait_for_low`, `wait_for_rising_edge`, …), `Output`. An input's level is the last edge the kernel gave; waiting registers a waker that an `advance` or `reply` carrying an edge wakes | `pin-config`, `pin-set`, edges |
| `spi::main` | `Spi` (`embedded_hal_async::spi::SpiBus`), with Ariel's `Frequency` and `Mode`. Each `read`, `write`, `transfer` and `transfer_in_place` is one `spi` request carrying the configured clock. `flush` is local | `spi` |
| `i2c::controller` | `I2c` (`embedded_hal_async::i2c::I2c`) with Ariel's `Frequency`. One `i2c` request per transaction; a NACK becomes `ErrorKind::NoAcknowledge` at the right address or data byte | `i2c` |
| `storage` | `Flash` (`embedded_storage_async::nor_flash::NorFlash`, `READ_SIZE` 1, `WRITE_SIZE` 4, `ERASE_SIZE` 4096) and `flash_range()` from `start` (ariel-sim `docs/m8.md` item 9) | `flash` |
| `identity` | `DeviceId` = the EUI-64 from `start` | – |
| `hwrng` | the RNG Ariel seeds its own from: `rand_chacha` seeded with `start.rng_seed` (D12), never the OS. `getrandom` calls in the firmware go to Ariel's RNG too, through Ariel's `hwrng` module's custom backend | – |

There is no `uart`, `usb` or `ble` module in M8. Apps that need them are unavailable on `sim`, and laze says so.

## Time, as the firmware sees it

- **Running code takes no time.** The clock moves only when the kernel grants a later time, or by the duration of a bus or flash operation, which the kernel's reply gives.
- **A request carries the node's clock** (`t`). The reply's `t` is when the operation ended, and the clock continues from there. So a driver polling a busy flag over SPI sees time pass by the bus time of each poll, as on hardware.
- **Before every message,** the HAL flushes stdout, so the console is in step (ariel-sim `docs/m8.md` item 11).
- **Busy-waiting on `Instant::now()` never ends** (ariel-sim `docs/m8.md` item 6). The supervisor catches it as a hang.

## Errors and limits

- A reply with `error` (an unknown pin or bus) panics with the kernel's message. That's a board or HAL bug, not something firmware can handle.
- A socket error or a message that doesn't decode panics too. The supervisor records it as a failure.
- Bus and flash operations over 32 KiB are split into several requests (`MAX_CHUNK`), so a message stays under 64 KiB.
- The flash's units are fixed, as `NorFlash` needs them at compile time: 1 MiB, 4-byte writes, 4 KiB erases (the sim board's). A board whose program flash differs gets `NotAligned` from the kernel.

## The messages

`src/wire` is a copy of ariel-sim's `sim-node-wire` crate: the message types generated from its `schema/node.cddl`, and their framing (JSON, one message per `SOCK_SEQPACKET` packet). ariel-sim is private and this fork is public, so the HAL can't depend on it by git. ariel-sim's `scripts/sync-wire.sh` writes the copy, and its CI checks the copy is current. Never edit it here.

## Dependencies

At the versions Ariel's workspace already uses: `embassy-executor`, `embassy-time-driver`, `embassy-time-queue-utils`, `embassy-hal-internal`, `embedded-hal`, `embedded-hal-async`, `embedded-storage-async`, `critical-section` (its `std` implementation: the node is one thread), `static_cell`, `rand_core`, `rand_chacha` (0.9, as `ariel-os-random` uses), `serde`, `serde_json`, and Ariel's own crates.

New to the workspace:
- `socket2`, for the `SOCK_SEQPACKET` socket, which `std` doesn't have;
- `base64` 0.22, for byte strings in the messages (the workspace has 0.21 through another crate).

## Tests

The crate builds only inside a `sim-chip` laze build (its `cfg(context)`s come from laze), so it has no `cargo test` of its own. It is tested through firmware, in ariel-sim: `sim-node/tests/m8a.rs` runs Ariel apps built for `-b sim` against the real kernel, covering the executor and time driver, GPIO and edges, SPI, I2C and its NACKs, storage, identity, the RNG, reboot and exit.

