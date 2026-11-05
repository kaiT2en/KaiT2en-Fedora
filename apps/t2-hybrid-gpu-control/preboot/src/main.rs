//! Restores the integrated boot GPU before Linux starts.
//!
//! macOS rewrites gpu-power-prefs on every boot, so the next Linux boot comes
//! up on the discrete GPU. The firmware reads the variable during POST, so
//! this image runs first, rewrites it when hybrid graphics is configured and
//! triggers a warm reset. Otherwise it chainloads shim right away.

#![no_main]
#![no_std]

mod font;

use core::mem::MaybeUninit;
use core::time::Duration;

use uefi::boot::{self, LoadImageSource, OpenProtocolAttributes, OpenProtocolParams, ScopedProtocol};
use uefi::proto::console::gop::{BltOp, BltPixel, GraphicsOutput};
use uefi::proto::device_path::build::{self, DevicePathBuilder};
use uefi::proto::device_path::DevicePath;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::BootPolicy;
use uefi::runtime::{self, ResetType, VariableAttributes, VariableVendor};
use uefi::{cstr16, guid, prelude::*, CStr16};

const APPLE_GPU_VARS: VariableVendor = VariableVendor(guid!("fa4ce28d-b62f-4c99-9cc3-6815686e30f9"));
const PREBOOT_VARS: VariableVendor = VariableVendor(guid!("cf9aa22b-9505-4c88-aed0-5673e31b00c2"));
const GPU_POWER_PREFS: &CStr16 = cstr16!("gpu-power-prefs");
/// Byte 0 is the wanted mode, byte 1 counts resets that did not take effect.
const BOOT_GPU: &CStr16 = cstr16!("boot-gpu");

const MODE_HYBRID: u8 = 1;
const PREFS_INTEGRATED: [u8; 4] = [1, 0, 0, 0];

const CHAINLOAD: [&CStr16; 2] = [
    cstr16!("\\EFI\\fedora\\shimx64.efi"),
    cstr16!("\\EFI\\fedora\\grubx64.efi"),
];

const RESTORE_TEXT: [&str; 2] = [
    "macOS switched the boot GPU back to the discrete GPU.",
    "Restoring hybrid graphics and restarting...",
];
const FAILED_TEXT: [&str; 2] = [
    "Hybrid graphics could not be restored.",
    "Starting Linux with the discrete GPU.",
];

fn attributes() -> VariableAttributes {
    VariableAttributes::NON_VOLATILE
        | VariableAttributes::BOOTSERVICE_ACCESS
        | VariableAttributes::RUNTIME_ACCESS
}

fn read_var<const N: usize>(name: &CStr16, vendor: &VariableVendor) -> Option<[u8; N]> {
    let mut buf = [0u8; N];
    let (data, _) = runtime::get_variable(name, vendor, &mut buf).ok()?;
    let data: [u8; N] = data.get(..N)?.try_into().ok()?;
    Some(data)
}

fn write_var(name: &CStr16, vendor: &VariableVendor, data: &[u8]) -> bool {
    runtime::set_variable(name, vendor, attributes(), data).is_ok()
}

/// Opens a protocol without taking it from the drivers that already use it.
fn get_protocol<P: uefi::proto::ProtocolPointer + ?Sized>(handle: Handle) -> Option<ScopedProtocol<P>> {
    let params = OpenProtocolParams {
        handle,
        agent: boot::image_handle(),
        controller: None,
    };
    // SAFETY: the protocol is only used while this image runs and nothing
    // here uninstalls it.
    unsafe { boot::open_protocol::<P>(params, OpenProtocolAttributes::GetProtocol) }.ok()
}

fn show(lines: &[&str]) {
    let Ok(handle) = boot::get_handle_for_protocol::<GraphicsOutput>() else {
        return;
    };
    let Some(mut gop) = get_protocol::<GraphicsOutput>(handle) else {
        return;
    };
    let (width, height) = gop.current_mode_info().resolution();
    let _ = gop.blt(BltOp::VideoFill {
        color: BltPixel::new(0, 0, 0),
        dest: (0, 0),
        dims: (width, height),
    });

    // The longest line of all messages takes at most 80 % of the width, so
    // every message renders at the same size.
    let longest = RESTORE_TEXT
        .iter()
        .chain(FAILED_TEXT.iter())
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(1);
    let scale = (height / 180)
        .min(width * 4 / 5 / (longest * font::ADVANCE))
        .max(1);
    let line_height = font::LINE_HEIGHT * scale;
    let block = line_height * lines.len();
    let mut y = height.saturating_sub(block) / 2;
    let color = BltPixel::new(0xe8, 0xe8, 0xe8);
    for line in lines {
        let text_width = line.chars().count() * font::ADVANCE * scale;
        let mut x = width.saturating_sub(text_width) / 2;
        for c in line.chars() {
            for (row, bits) in font::glyph(c).iter().enumerate() {
                for col in 0..font::WIDTH {
                    if bits & (1 << (font::WIDTH - 1 - col)) == 0 {
                        continue;
                    }
                    let _ = gop.blt(BltOp::VideoFill {
                        color,
                        dest: (x + col * scale, y + row * scale),
                        dims: (scale, scale),
                    });
                }
            }
            x += font::ADVANCE * scale;
        }
        y += line_height;
    }
}

/// Wanted state is hybrid, returns whether a reset is needed.
fn restore_integrated(state: [u8; 2]) -> bool {
    let prefs = read_var::<4>(GPU_POWER_PREFS, &APPLE_GPU_VARS);
    if prefs == Some(PREFS_INTEGRATED) {
        if state[1] != 0 {
            write_var(BOOT_GPU, &PREBOOT_VARS, &[MODE_HYBRID, 0]);
        }
        return false;
    }

    match state[1] {
        0 => {
            // Count the attempt first so a firmware that ignores the
            // variable cannot reset in a loop.
            if !write_var(BOOT_GPU, &PREBOOT_VARS, &[MODE_HYBRID, 1])
                || !write_var(GPU_POWER_PREFS, &APPLE_GPU_VARS, &PREFS_INTEGRATED)
            {
                return false;
            }
            show(&RESTORE_TEXT);
            boot::stall(Duration::from_secs(3));
            true
        }
        1 => {
            write_var(BOOT_GPU, &PREBOOT_VARS, &[MODE_HYBRID, 2]);
            show(&FAILED_TEXT);
            boot::stall(Duration::from_secs(5));
            false
        }
        _ => false,
    }
}

fn chainload() -> Status {
    let Some(device) = get_protocol::<LoadedImage>(boot::image_handle()).and_then(|image| image.device())
    else {
        return Status::NOT_FOUND;
    };
    let Some(device_path) = get_protocol::<DevicePath>(device) else {
        return Status::NOT_FOUND;
    };

    let mut status = Status::NOT_FOUND;
    for file in CHAINLOAD {
        let mut storage = [MaybeUninit::<u8>::uninit(); 1024];
        let mut builder = DevicePathBuilder::with_buf(&mut storage);
        for node in device_path.node_iter() {
            let Ok(next) = builder.push(&node) else {
                return Status::BUFFER_TOO_SMALL;
            };
            builder = next;
        }
        let Ok(path) = builder
            .push(&build::media::FilePath { path_name: file })
            .and_then(|builder| builder.finalize())
        else {
            return Status::BUFFER_TOO_SMALL;
        };

        let source = LoadImageSource::FromDevicePath {
            device_path: path,
            boot_policy: BootPolicy::ExactMatch,
        };
        match boot::load_image(boot::image_handle(), source) {
            Ok(image) => return boot::start_image(image).status(),
            Err(error) => status = error.status(),
        }
    }
    // An error return lets the firmware continue with the next BootOrder
    // entry, which is the regular Fedora entry.
    status
}

#[entry]
fn main() -> Status {
    if let Some(state) = read_var::<2>(BOOT_GPU, &PREBOOT_VARS)
        && state[0] == MODE_HYBRID
        && restore_integrated(state)
    {
        runtime::reset(ResetType::WARM, Status::SUCCESS, None);
    }
    chainload()
}
