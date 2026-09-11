# KaiT2en t2-journal

t2-journal can fetch Apple T2 bridgeOS logs and merge them with a selected 
Linux boot. The application keeps one parsed BridgeOS snapshot until you run
`t2journal refresh`. A refresh atomically replaces the kept T2 logs. 

## T2 link

The T2 is reached over the `Apple T2 Bridge` NetworkManager profile that
`scripts/fedora/install-t2-remote.sh` creates on the internal CDC-NCM
interface. `t2remote`, built from this crate and run by
`kait2en-t2-remote.service`, holds the RemoteXPC services and disconnects
the link around suspend. No manual network setup is needed.

## Build & Install

```bash
make && sudo make install
```

## Usage

```bash
t2journal -b
t2journal -b -1
t2journal --allboots
t2journal -b --all
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
when their timestamps fall inside that interval. `--allboots` (also accepted as
`--all-boots`) disables the T2 window filter and merges the complete snapshot
with all retained Linux boots. RemoteXPC transport messages and explicit state
dumps caused by collection are hidden by default; `--all` includes them.
The built-in `--grep` also searches process, subsystem, category, and source.
Like `journalctl`, an all-lowercase pattern is matched case-insensitively; a
pattern containing uppercase letters is matched case-sensitively.
Text output uses exactly one physical line per record; embedded line endings and
backslashes are escaped as `\n`, `\r`, and `\\` without discarding their content.

The default snapshot is
`$XDG_STATE_HOME/t2-journal/bridgeos.jsonl`, falling back to
`~/.local/state/t2-journal/bridgeos.jsonl`. A refresh verifies this destination
before contacting the T2, writes one fixed `bridgeos.jsonl.partial`, and
atomically replaces the snapshot after a successful parse.
RemoteXPC discovery first connects directly to the T2's fixed port `59602`
and reads the current sysdiagnose service port from its service directory.
If direct discovery fails, it falls back to a dynamic-port scan because we don't know yet if `59602` is universal.
