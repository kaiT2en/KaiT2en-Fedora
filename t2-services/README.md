# T2 services

Each feature owns its implementation, integration and packaging. There is no
dependency between the three feature packages.

| Source | Binary package | Installed command | Build dependencies within this tree |
| --- | --- | --- | --- |
| `t2-touchid/` | `t2-touchid` | `t2-touchid` | its `protocols/t2-biometrickit`, shared BridgeXPC |
| `t2-ave/` | `t2-ave` | `t2remote` (existing interface) | shared BridgeXPC |
| `t2-journal/` | `t2-journal` | `t2journal` (existing interface) | shared BridgeXPC, its `libs/macos-unifiedlogs` |
| `shared/` | `t2-services-common` | none | none |

`shared/protocols/t2-bridgexpc` is a Rust build dependency, linked into consumers.
`t2-services-common` installs only the common runtime integration. All three feature
packages depend on that package for access to the internal CDC-NCM link.

## Layout

`cli/` contains command-line handling, `daemon/` the long-running service,
`protocols/` feature-specific wire protocols, `libs/` supporting libraries,
`kernel/` kernel code, `config/` application defaults, and `integration/` files
consumed by systemd, D-Bus, fprintd, SELinux or NetworkManager. Only directories
with an implementation are present. Each component retains its own Cargo lockfile.
Building one does not resolve the dependencies of unrelated features.

## Build and stage

From any t2-services feature directory:

```sh
make build
make test
make install PREFIX=/usr DESTDIR=/tmp/t2-package
```

Shared integration uses `make -C shared install` with the same prefix and staging
variables. Installation never starts services, runs modprobe, changes PAM or
talks to the live network. Systemd paths are substituted when staging, so source
installs can use `/usr/local` and distribution packages `/usr`. Configuration
already present in the destination is preserved. SELinux policy has a separate
build in `t2-touchid/integration/selinux` for distributions that use it.

Each `packaging/` directory contains RPM and Debian recipes. See
`shared/packaging/README.md` for source preparation and platform limits.

## Suspend and resume

`shared/integration/systemd/t2-services-suspend.service` runs feature sleep/resume
hooks around system suspend. The T2 NCM link survives suspend on its own
(`t2bce_vhci` reset-resumes it after a stateful sleep), so the helper no longer
touches it. The AVE hook closes/reopens only its own sessions, and skips an
inactive AVE daemon. Touch ID uses its existing logind resume watcher and
connection retry loop. Journal holds no persistent daemon session.

The remaining `kait2en-suspend.service` handles WLAN/Bluetooth only.

## Kernel boundary

AVE kernel sources live in `t2-ave/kernel/t2bce_ave`. The existing Fedora DKMS
installer still stages them together with `modules/t2bce_stack` so the shared
symbol namespace is preserved. The userspace package does not bundle a `.ko`.
Distributions must provide a compatible `t2bce_ave` and BCE core, either in their
kernel or via a separately maintained kernel-module package. This is not a
dependency on Touch ID or Journal.
