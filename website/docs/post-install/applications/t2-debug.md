# T2 Debug

<img class="app-shot" src="img/apps/t2-debug.webp" alt="T2 Debug streaming the bridgeOS live log with the subsystem filter">

Shows live data from bridgeOS, the T2's own operating system. It connects over
the internal T2 network link, the same one [T2 Journal](t2-journal.md) uses,
to the T2's Instruments diagnostic service. It only reads.

Two tabs:

- **deviceinfo** is a "top" for the T2: every bridgeOS process with its CPU,
  power score, interrupt wakeups, memory footprint and disk writes, updated once
  a second. Click a column header to sort. Buttons run one-shot queries (mach
  time, network info, the kdebug trace-code table, the full service capability
  list) and list a bridgeOS directory.
- **live log** streams the T2's `os_log` in real time, decoded to readable
  lines and coloured by level like `dmesg`. A regex box filters, a "hide HID"
  toggle drops the input-device chatter, and the subsystem list on the right
  lets you tick exactly which subsystems to show.

This is the tool for watching what the T2 does in the moment, for example the
SMC and power events around a GPU switch or a suspend. Logs the T2 marks as
private appear as `<private>`. bridgeOS redacts them before they reach Linux.

The command `t2debug` gives the same data without a GUI, for example over SSH (`t2debug procs`, `t2debug logs --no-hid`,
`t2debug sysmon`). Run `t2debug --help` for all subcommands.
