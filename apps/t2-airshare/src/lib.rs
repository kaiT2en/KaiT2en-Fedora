// SPDX-License-Identifier: GPL-3.0-or-later
//! T2 AirShare: Quick Share (Nearby Share) and AirDrop for Linux on T2
//! Macs. Quick Share runs here, AirDrop through the separate t2-drop
//! service over D-Bus.
//!
//! The Quick Share protocol follows NearDrop's PROTOCOL.md and the Nearby protobuf
//! definitions. Parts of the UKEY2 handshake and the secure channel are
//! adapted from rquickshare, see THIRD-PARTY-NOTICES.md.

pub mod airdrop;
pub mod config;
pub mod discover;
pub mod endpoint;
pub mod link;
pub mod mdns;
pub mod notify;
pub mod proto;
pub mod receive;
pub mod secure;
pub mod send;
pub mod service;
