//! Records the target triple this crate is compiled for, as
//! `BLENTINEL_TARGET` (read by `release::Platform::current`).
//! `std::env::consts::ARCH` says `arm` for ARMv6 and ARMv7 alike, and Rust's
//! ARM target features can't be tested with `cfg!` on stable: only the
//! triple tells a Pi Zero's build from a Pi 3's.

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=BLENTINEL_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
