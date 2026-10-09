//! `cfg(doctors)` (PUB-1a item 3): the acceptance doctors and the fixture-backed tests are
//! compiled when the cargo feature `doctors` is on and their modules are present
//! (src/doctors/, private: the public export does not carry it). Without either, `palace doctor
//! --acceptance` reports that this build has no acceptance doctors.
use std::{env, path::Path};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(doctors)");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src");
    let doctors = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/doctors/mod.rs");
    if env::var_os("CARGO_FEATURE_DOCTORS").is_some() && doctors.is_file() {
        println!("cargo::rustc-cfg=doctors");
    }
}
