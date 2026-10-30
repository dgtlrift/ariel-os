//! The chip this build stands in for (ariel-sim, D40): one `sim_chip` cfg from the build's
//! `context` cfgs, so the HAL offers that chip's peripheral names. A board built with
//! `sim-hardware` whose chip has no version here is `sim_chip = "unsupported"`, which
//! stops the build with a message naming the chips that have one.

/// The chips with a sim version, most specific context first.
const CHIPS: &[&str] = &["nrf52840", "esp32s3", "esp32s2", "esp32"];

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_CONTEXT");
    println!("cargo:rerun-if-env-changed=CARGO_CFG_SIM_HARDWARE");
    let values: Vec<String> = CHIPS
        .iter()
        .map(|c| format!("\"{c}\""))
        .chain(["\"generic\"".into(), "\"unsupported\"".into()])
        .collect();
    println!("cargo::rustc-check-cfg=cfg(sim_chip, values({}))", values.join(", "));
    let contexts = std::env::var("CARGO_CFG_CONTEXT").unwrap_or_default();
    let contexts: Vec<&str> = contexts.split(',').collect();
    let chip = CHIPS.iter().find(|c| contexts.contains(c)).copied().unwrap_or(
        // The `sim` board, and native builds, are the generic chip.
        if std::env::var_os("CARGO_CFG_SIM_HARDWARE").is_some() && !contexts.contains(&"sim") {
            "unsupported"
        } else {
            "generic"
        },
    );
    println!("cargo:rustc-cfg=sim_chip=\"{chip}\"");
}
