// SPDX-License-Identifier: GPL-3.0-or-later
//! T2 TrueTone: ambient white point adaptation, evening mode and automatic
//! brightness for the built-in panel of T2 MacBooks under GNOME.

pub mod als;
pub mod autobright;
pub mod config;
pub mod daemon;
pub mod gamma;
pub mod gnome;
pub mod model;
pub mod mutter;
pub mod night;
pub mod status;

pub const SERVICE: &str = "t2-truetone.service";
