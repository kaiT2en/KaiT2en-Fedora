// SPDX-License-Identifier: GPL-3.0-or-later
//! Generated protobuf types, nested like their protobuf packages so the
//! cross-package paths prost emits resolve.

#![allow(clippy::all)]

pub mod location {
    pub mod nearby {
        pub mod connections {
            include!(concat!(env!("OUT_DIR"), "/location.nearby.connections.rs"));
        }
    }
}

pub mod sharing {
    pub mod nearby {
        include!(concat!(env!("OUT_DIR"), "/sharing.nearby.rs"));
    }
}

pub mod securegcm {
    include!(concat!(env!("OUT_DIR"), "/securegcm.rs"));
}

pub mod securemessage {
    include!(concat!(env!("OUT_DIR"), "/securemessage.rs"));
}
