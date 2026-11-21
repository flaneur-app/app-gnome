// SPDX-License-Identifier: AGPL-3.0-or-later

//! Compiles the Blueprint files of `data/ui` to GtkBuilder XML, then bundles them with the other
//! files of `data/` into one GResource that `main.rs` embeds in the binary.
//!
//! Needs `blueprint-compiler` and `glib-compile-resources` on the `PATH` (Debian:
//! `blueprint-compiler`, `libglib2.0-dev-bin`; both ship in the GNOME Flatpak SDK).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    println!("cargo::rerun-if-changed=data");
    println!("cargo::rerun-if-env-changed=FLANEUR_APP_ID");

    let mut blueprints: Vec<PathBuf> = fs::read_dir("data/ui")
        .expect("data/ui exists")
        .map(|entry| entry.expect("data/ui is readable").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "blp"))
        .collect();
    blueprints.sort();

    let status = Command::new("blueprint-compiler")
        .arg("batch-compile")
        .arg(out_dir.join("ui"))
        .arg("data/ui")
        .args(&blueprints)
        .status()
        .expect("blueprint-compiler must be installed (Debian package: blueprint-compiler)");
    assert!(status.success(), "blueprint-compiler failed");

    glib_build_tools::compile_resources(
        &[out_dir.as_path(), Path::new("data")],
        "data/resources.gresource.xml",
        "flaneur.gresource",
    );
}
