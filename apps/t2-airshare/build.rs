// SPDX-License-Identifier: GPL-3.0-or-later
//! Compiles the Nearby protobuf definitions in `proto/` with protox, so no
//! protoc is needed at build time.

const PROTOS: [&str; 6] = [
    "device_to_device_messages.proto",
    "offline_wire_formats.proto",
    "securegcm.proto",
    "securemessage.proto",
    "ukey.proto",
    "wire_format.proto",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for proto in PROTOS {
        println!("cargo:rerun-if-changed=proto/{proto}");
    }
    let files = PROTOS.map(|proto| format!("proto/{proto}"));
    let descriptors = protox::compile(files, ["proto"])?;
    prost_build::Config::new().compile_fds(descriptors)?;
    Ok(())
}
