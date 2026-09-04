# KaiT2en t2-journal

t2-journal can fetch Apple T2 bridgeOS logs and merge them with a selected 
Linux boot. The application keeps one parsed BridgeOS snapshot until you run
`t2journal refresh`. A refresh atomically replaces the kept T2 logs. 

## Manage NCM Connection

Run `scripts/fedora/install-networkmanager-rules.sh` to remove services we 
implemented to keep ncm hidden. The run 
```bash
sudo rm /etc/udev/rules.d/90-kait2en-t2-network.rules
reboot
```

After reboot go to your network settings and connect ncm with IPv6 link local.
Deactivate IPv4 for ncm. Since this t2-journal is still WIP, problems are
expected. Like the computer refusing to suspend because t2bce is not yet
prepared. Release or deactivate the connection to be able to suspend.
Or revert to previous state by running `scripts/fedora/install-networkmanager-rules.sh`.
It will reinstall the udev rule to unmanage ncm.

## Build & Install

```bash
make && sudo make install
```

## Usage

```bash
t2journal -b
t2journal -b -1
t2journal --list-boots
t2journal -b --grep 'suspend|watchdog'
t2journal -b --grep=smc
t2journal -b --output jsonl > merged.jsonl
t2journal -b > t2journal.log
```

If no T2 log snapshot exists, a query performs the initial refresh
automatically.

Only records are written to stdout. Progress and errors are written to stderr,
so redirection and pipelines behave normally. Boot indices select the Linux
journal's UTC interval. BridgeOS records from the current snapshot are included
when their timestamps fall inside that interval.

The default snapshot is
`$XDG_STATE_HOME/t2-journal/bridgeos.jsonl`, falling back to
`~/.local/state/t2-journal/bridgeos.jsonl`. A refresh verifies this destination
before contacting the T2, writes one fixed `bridgeos.jsonl.partial`, and
atomically replaces the snapshot after a successful parse.