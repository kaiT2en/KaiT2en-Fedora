# t2-ave

The T2 video encoders live here: the userspace service, its protocol and the
kernel modules `t2-ave` and `t2-sidecar`. The existing `t2remote` command
and `kait2en-t2-remote.service` names remain available to preserve installed
configurations and suspend clients.

The parts follow the macOS components:

| macOS | here |
| --- | --- |
| `remoted` | `t2remote`, holds `com.apple.aveservice` and `com.apple.videoprocessingd.encode.remote` |
| `AppleAVEBridge` | `kernel/t2-ave`, AVE over the T2 bridge queues, V4L2 HEVC encoder `apple-ave-enc` |
| `VCPSideCarCompressionSession`, `VCPSideCarUSBSession` | `kernel/t2-sidecar`, V4L2 H.264/HEVC encoder `t2-sidecar` |

The T2 exposes the Sidecar encoder as a virtual USB device (`05ac:9312`, "SideCar
AVE") only while a client holds `com.apple.videoprocessingd.encode.remote`.
`t2remote` connects it at start after AVE, and `t2-sidecar` binds to the device
through `apple-t2-vhci-hcd`. On macOS the Sidecar client runs in userspace inside
VideoProcessing.framework. Here it is a kernel driver, because a V4L2 encoder needs
one.

`daemon/` manages service sessions, `protocols/` owns the AVE start/stop messages,
and `integration/` contains the unit and the hook installed into the common
NCM suspend helper. NetworkManager and NCM binding belong to `../shared/`.

Build and install userspace with `make build` and `make install`. Use `PREFIX`
and `DESTDIR` for packaging. Building the kernel module requires matching kernel
headers and the symbols of `apple-t2-bridge`. The Fedora DKMS package
`apple-t2-bridge` stages `kernel/t2-ave/` and `kernel/t2-sidecar/` into its tree.
