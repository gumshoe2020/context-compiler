//! The owner's names, compiled in as the default `--person` and `--assistant` (PUB-1a item 2):
//! from $PALACE_OWNER_PERSON and $PALACE_OWNER_ASSISTANT, else the private tree's
//! dev/owner-defaults (`PERSON=<name>` and `ASSISTANT=<name>` lines). With neither there are no
//! defaults (`names::OWNER` is `None`). A build script reads them, not the source, so that no
//! owner's name is written in a source file.
use std::{env, fs, path::Path};

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=PALACE_OWNER_PERSON");
    println!("cargo::rerun-if-env-changed=PALACE_OWNER_ASSISTANT");
    let file = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../dev/owner-defaults");
    let text = if file.is_file() {
        println!("cargo::rerun-if-changed={}", file.display());
        fs::read_to_string(&file).unwrap_or_default()
    } else {
        String::new()
    };
    for (key, var) in [
        ("PERSON", "PALACE_OWNER_PERSON"),
        ("ASSISTANT", "PALACE_OWNER_ASSISTANT"),
    ] {
        let name = env::var(var).ok().or_else(|| line_value(&text, key));
        if let Some(name) = name {
            println!("cargo::rustc-env={var}={name}");
        }
    }
}

/// The value of the `<key>=<value>` line in `text`, if any.
fn line_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.trim().split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.trim().to_owned())
}
