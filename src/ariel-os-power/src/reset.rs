/// Reboots the MCU.
///
/// This function initiates a software reset of the microcontroller and never returns.
pub fn reboot() -> ! {
    cfg_select! {
        context = "cortex-m" => cortex_m::peripheral::SCB::sys_reset(),
        // The simulator restarts the node's process when it exits with status 3.
        context = "sim-chip" => std::process::exit(3),
        context = "esp" => esp_hal::system::software_reset(),
        context = "native-chip" => std::process::exit(0),
        context = "ariel-os" => {
            compile_error!("reboot is not yet implemented for this platform")
        }
        _ =>
        {
            #[expect(clippy::empty_loop, reason = "for platform-independent tooling only")]
            loop {}
        }
    }
}
