// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (c) 2023 WhatAmISupposedToPutHere
//
// The DRM setup is derived from tiny-dfr's MIT-licensed display backend:
// https://github.com/AsahiLinux/tiny-dfr

use std::{
    fs::{File, OpenOptions},
    os::fd::{AsFd, BorrowedFd},
    path::Path,
};

use anyhow::{Context, Result, anyhow};
use drm::{
    ClientCapability, Device as DrmDevice,
    buffer::DrmFourcc,
    control::{
        AtomicCommitFlags, ClipRect, Device as ControlDevice, Mode, ResourceHandle, atomic,
        connector,
        dumbbuffer::{DumbBuffer, DumbMapping},
        framebuffer, property,
    },
};

struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl ControlDevice for Card {}
impl DrmDevice for Card {}

impl Card {
    fn open(path: &Path) -> Result<Self> {
        Ok(Self(OpenOptions::new().read(true).write(true).open(path)?))
    }
}

pub struct Display {
    card: Card,
    mode: Mode,
    buffer: DumbBuffer,
    framebuffer: framebuffer::Handle,
}

impl Drop for Display {
    fn drop(&mut self) {
        let _ = self.card.destroy_framebuffer(self.framebuffer);
        let _ = self.card.destroy_dumb_buffer(self.buffer);
    }
}

fn property_id<T: ResourceHandle>(card: &Card, handle: T, name: &str) -> Result<property::Handle> {
    let props = card.get_properties(handle)?;
    for id in props.as_props_and_values().0 {
        let info = card.get_property(*id)?;
        if info.name().to_str()? == name {
            return Ok(*id);
        }
    }
    Err(anyhow!("DRM property {name} not found"))
}

fn try_open(path: &Path) -> Result<Display> {
    let card = Card::open(path)?;
    card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
    card.set_client_capability(ClientCapability::Atomic, true)?;
    card.acquire_master_lock()?;

    let resources = card.resource_handles()?;
    let connector = resources
        .connectors()
        .iter()
        .filter_map(|handle| card.get_connector(*handle, true).ok())
        .find(|info| info.state() == connector::State::Connected)
        .ok_or_else(|| anyhow!("no connected connector"))?;
    let mode = *connector
        .modes()
        .first()
        .ok_or_else(|| anyhow!("connector has no mode"))?;
    let (physical_width, physical_height) = mode.size();
    if physical_height / physical_width < 30 {
        return Err(anyhow!("connector is not Touch-Bar-shaped"));
    }

    let crtc = resources
        .crtcs()
        .first()
        .and_then(|handle| card.get_crtc(*handle).ok())
        .ok_or_else(|| anyhow!("no CRTC"))?;
    let plane = *card
        .plane_handles()?
        .first()
        .ok_or_else(|| anyhow!("no plane"))?;
    let buffer = card.create_dumb_buffer((64, physical_height.into()), DrmFourcc::Xrgb8888, 32)?;
    let framebuffer = card.add_framebuffer(&buffer, 24, 32)?;

    let mut request = atomic::AtomicModeReq::new();
    request.add_property(
        connector.handle(),
        property_id(&card, connector.handle(), "CRTC_ID")?,
        property::Value::CRTC(Some(crtc.handle())),
    );
    let mode_blob = card.create_property_blob(&mode)?;
    request.add_property(
        crtc.handle(),
        property_id(&card, crtc.handle(), "MODE_ID")?,
        mode_blob,
    );
    request.add_property(
        crtc.handle(),
        property_id(&card, crtc.handle(), "ACTIVE")?,
        property::Value::Boolean(true),
    );
    request.add_property(
        plane,
        property_id(&card, plane, "FB_ID")?,
        property::Value::Framebuffer(Some(framebuffer)),
    );
    request.add_property(
        plane,
        property_id(&card, plane, "CRTC_ID")?,
        property::Value::CRTC(Some(crtc.handle())),
    );
    for (name, value) in [
        ("SRC_X", 0),
        ("SRC_Y", 0),
        ("SRC_W", (physical_width as u64) << 16),
        ("SRC_H", (physical_height as u64) << 16),
    ] {
        request.add_property(
            plane,
            property_id(&card, plane, name)?,
            property::Value::UnsignedRange(value),
        );
    }
    for (name, value) in [("CRTC_X", 0), ("CRTC_Y", 0)] {
        request.add_property(
            plane,
            property_id(&card, plane, name)?,
            property::Value::SignedRange(value),
        );
    }
    for (name, value) in [
        ("CRTC_W", physical_width as u64),
        ("CRTC_H", physical_height as u64),
    ] {
        request.add_property(
            plane,
            property_id(&card, plane, name)?,
            property::Value::UnsignedRange(value),
        );
    }
    card.atomic_commit(AtomicCommitFlags::ALLOW_MODESET, request)?;
    Ok(Display {
        card,
        mode,
        buffer,
        framebuffer,
    })
}

impl Display {
    pub fn open() -> Result<Self> {
        let path = crate::usb::touchbar_drm_card()
            .ok_or_else(|| anyhow!("Touch Bar DRM device is not attached"))?;
        try_open(&path).with_context(|| format!("open Touch Bar DRM device {}", path.display()))
    }

    /// Logical dimensions after applying the panel's 90-degree orientation.
    pub fn dimensions(&self) -> (u16, u16) {
        let (height, width) = self.mode.size();
        (width, height)
    }

    fn map(&mut self) -> Result<DumbMapping<'_>> {
        Ok(self.card.map_dumb_buffer(&mut self.buffer)?)
    }

    pub fn present(&mut self, logical: &[u32], width: u16, height: u16) -> Result<()> {
        self.present_columns(logical, width, height, 0, width)
    }

    /// Sends only the columns from `left` to `right`. The rest of the panel
    /// keeps the previous frame, which must match `logical` there.
    pub fn present_columns(
        &mut self,
        logical: &[u32],
        width: u16,
        height: u16,
        left: u16,
        right: u16,
    ) -> Result<()> {
        anyhow::ensure!(
            logical.len() == width as usize * height as usize,
            "bad frame size"
        );
        let (left, right) = (left.min(width), right.min(width));
        let info = self.card.get_framebuffer(self.framebuffer)?;
        let stride = info.pitch() as usize;
        let mut mapping = self.map()?;
        let target = mapping.as_mut();

        // t2bdrm exposes the panel rotated. This is the same transform as
        // tiny-dfr's cairo translate(height, 0) + rotate(90deg).
        for y in 0..height as usize {
            for x in left as usize..right as usize {
                let physical_x = height as usize - 1 - y;
                let physical_y = x;
                let offset = physical_y * stride + physical_x * 4;
                target[offset..offset + 4]
                    .copy_from_slice(&logical[y * width as usize + x].to_le_bytes());
            }
        }
        drop(mapping);
        // t2bdrm sends only the damaged rectangle over USB.
        self.card
            .dirty_framebuffer(self.framebuffer, &[ClipRect::new(0, left, height, right)])?;
        Ok(())
    }
}
