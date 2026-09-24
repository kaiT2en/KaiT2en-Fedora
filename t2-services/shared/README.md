# Shared T2 infrastructure

This directory owns the transport and system integration used by `t2-touchid`,
`t2-ave` and `t2-journal`.

| Directory | Responsibility |
| --- | --- |
| `protocols/t2-bridgexpc/` | Rust library for discovery, RemoteXPC and BridgeXPC over the internal CDC-NCM link |
| `integration/NetworkManager/` | IPv6 link-local connection profile and prevention of automatically generated duplicate profiles |
| `integration/systemd/` | Common suspend/resume unit |
| `integration/libexec/` | Feature sleep/resume hook runner and error collection for package lifecycle actions |
| `packaging/` | RPM and Debian recipes for `t2-services-common` |

## Dependencies

BridgeXPC is a build dependency linked into each consumer. Its source stays here
so all three features use the same transport implementation.

The `t2-services-common` binary package contains the shared runtime integration.
It depends on NetworkManager, systemd and the required command-line utilities.
Each feature package depends on it. The common package does not depend on any
feature package. Feature protocols and daemons remain in their own directories.

## Suspend and resume

`t2-services-suspend.service` runs `t2-ncm-sleep pre` before sleep and
`t2-ncm-sleep post` on resume. Despite the name, the helper no longer touches
the T2 NCM link itself: `t2bce_vhci` reset-resumes it in-kernel after a
stateful sleep, so it survives suspend without host-side unbind/rebind. The
helper's only remaining job is running installed feature hooks with `pre` and
`post`.

Executable hooks live in `libexec/t2-services/sleep.d/` below the installation
prefix. AVE installs its own hook there. Touch ID uses its logind resume watcher
and connection retries. Journal does not have a persistent daemon. The common
helper also works with no feature hooks installed.

## Build and installation

The integration files need no compilation. From this directory, stage them for
a distribution package with:

```sh
make install PREFIX=/usr DESTDIR=/tmp/t2-common-package
```

`LIBEXECDIR`, `SYSTEMD_UNIT_DIR` and `SYSCONFDIR` can be overridden. The default
prefix is `/usr/local`. Installation substitutes the helper path in the systemd
unit and preserves an existing `t2-ncm.nmconnection`. It does not start services
or change the live network.

The Fedora source installer uses `INSTALL_NETWORK_PROFILE=no` while migrating
existing profiles through `scripts/fedora/install-t2-services-common.sh`.
Network profile migration belongs to that installer. WLAN and Bluetooth
handling remain separate.

Build or test the transport library independently with:

```sh
cargo build --locked --manifest-path protocols/t2-bridgexpc/Cargo.toml
cargo test --locked --manifest-path protocols/t2-bridgexpc/Cargo.toml
```

See [packaging/README.md](packaging/README.md) for package builds, activation and
migration from existing source installations. Package lifecycle errors are
summarized and written to `/var/log/t2-services-install.log`.
