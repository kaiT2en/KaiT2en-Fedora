# T2 Journal

Fetches Apple T2 bridgeOS logs over the internal T2 network
link and merges them chronologically with the Linux journal. It can select one
boot or all retained boots, filter by regular expression or source, and emit
text or JSONL. The first query downloads a BridgeOS snapshot automatically. Use
`t2journal refresh` to replace it explicitly.

The link is set up by the installer and managed by `kait2en-t2-remote`, the
same service that holds the T2 video encoder open. It is disconnected before
suspend and reconnected after resume, so there is nothing to configure.

Typical queries are:

```bash
t2journal -b
t2journal -b -1 --grep 'suspend|watchdog'
t2journal --allboots --source t2
t2journal -b --output jsonl > merged.jsonl
```

`t2journal refresh --sysdiagnose` additionally keeps a copy of the downloaded
sysdiagnose archive in the current directory, e.g. to extract panic logs from
it yourself.

Run `t2journal --help` for all filtering and refresh options.
