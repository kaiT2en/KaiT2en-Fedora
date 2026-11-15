# KaiT2en Update

<p align="center">
<img src="org.kait2enupdate.gtk.svg" alt="KaiT2en Update logo" width="96">
</p>

KaiT2en Update fetches `origin/main` into `/usr/local/src/KaiT2en-Fedora`,
reads the map from GitHub and compares each record with it. A step runs
again when one of its sources changed since its record.

| Change | Runs |
|---|---|
| `modules/t2smc/t2smc.c` | DKMS modules and the initramfs, then a reboot |
| `apps/t2-fan-control/src/main.rs` | only T2 Fan Control |
| `assets/vornehm/...` | only KaiT2en Vornehm, where it is installed |
| `README.md`, `website/` | nothing |
| `scripts/fedora/lib.sh`, `install.sh` | a full installation is offered |

Parts without a record yet, for example after an update from an older
release, are compared with the local checkout. A record with a commit the
checkout does not know runs again.

The update itself runs through `pkexec kait2en-update-helper`, which only
accepts names from the map and calls

```bash
install.sh --only step,step
KAIT2EN_APPS=app,app install.sh --only install-apps.sh
```

Optional parts like the Touch Bar, the KaiT2en login screen and KaiT2en
Vornehm stay as they are. They are added or removed in the Optional section.

## Adding a step or an app

Add a line to `scripts/fedora/update-map` with its sources, in installation
order. An `install-*.sh` script depends on itself automatically. Apps
installed by `install-apps.sh` are named `app:<name>` and need a matching
`wanted <name>` there.

## Install

Installed with the other apps by `scripts/fedora/install-apps.sh`.
