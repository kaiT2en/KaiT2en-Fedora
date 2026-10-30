# Third-party notices

## Protobuf definitions

The files in `proto/` are the Nearby Connections and Nearby Sharing
protocol definitions, taken from rquickshare `core_lib/src/proto_src`
(commit 378d8ae), which collected them from the Chromium sources.

- `device_to_device_messages.proto`, `offline_wire_formats.proto`,
  `securegcm.proto`, `securemessage.proto`, `ukey.proto`: Copyright 2020
  Google LLC, Apache License 2.0, see `proto/LICENSE-Apache-2.0`
- `wire_format.proto`: Copyright 2020 The Chromium Authors, BSD-style
  license, see `proto/LICENSE-Chromium`

The license headers in the files are kept unchanged.

## rquickshare

Parts of `src/secure.rs`, `src/link.rs`, `src/receive.rs`, `src/send.rs`
and `src/discover.rs` (UKEY2 key derivation, the secure message layout, the
PIN algorithm, the payload framing, the receiving and sending frame
sequences and the BLE advertisement) are adapted from rquickshare by
martichou (https://github.com/Martichou/rquickshare,
`core_lib/src/hdl/inbound.rs`, `core_lib/src/hdl/outbound.rs`,
`core_lib/src/hdl/blea.rs` and `core_lib/src/utils.rs`, commit 378d8ae),
licensed under the GNU General Public License version 3. The combined program is distributed under
GPL-3.0-or-later, see `LICENSE`.

## Protocol documentation

The layout of the BLE Fast Initiation advertisement follows
`sharing/internal/api/fast_init_ble_beacon.h` in google/nearby
(https://github.com/google/nearby, Apache License 2.0). No code is copied
from it.

The protocol description follows NearDrop's `PROTOCOL.md` by grishka
(https://github.com/grishka/NearDrop), released into the public domain
under the Unlicense.
