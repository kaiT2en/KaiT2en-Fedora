use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=resources.gresource.xml");
    println!("cargo:rerun-if-changed=assets/icons/hicolor/index.theme");
    println!("cargo:rerun-if-changed=assets/icons/hicolor/scalable/apps/org.t2smccontrol.gtk.svg");

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"));
    let output = out_dir.join("t2-smc-control.gresource");

    let status = Command::new("glib-compile-resources")
        .args([
            "resources.gresource.xml",
            "--sourcedir=.",
            "--target",
        ])
        .arg(&output)
        .status()
        .expect("failed to execute glib-compile-resources");

    if !status.success() {
        panic!("glib-compile-resources failed with status: {status}");
    }
}
