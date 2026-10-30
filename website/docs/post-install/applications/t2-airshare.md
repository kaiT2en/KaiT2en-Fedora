# T2 AirShare

T2 AirShare shares files with Apple devices over AirDrop and with Android
phones over Quick Share, both in one app. Air from AirDrop, Share from
Quick Share. Receiving and sending work the same way for both.

- Open T2 AirShare and switch on "Visible to nearby devices". This starts the
  user service `t2-airshare.service`, which also runs after the next login.
- Files go to the Downloads folder, the window lets you choose another one.
  "Show in folder" in the notification opens the folder with the files
  selected.
- To send, right-click files in the file manager and choose "Send with T2
  AirShare" (Files, Dolphin, Nemo), use "Open With" in other file managers,
  or drop files on the T2 AirShare window. Apple devices and Android phones
  show up in the same list.

## AirDrop

AirDrop runs over AWDL, the Wi-Fi link Apple devices open between each
other, next to the normal Wi-Fi connection. KAIT2EN drives AWDL in the
Apple firmware of the T2 Macs' Broadcom Wi-Fi chip through its patched
brcmfmac driver and the system service `t2-drop.service`. The Mac stays
connected to its Wi-Fi network while AirDrop runs. Bluetooth has to be on,
Apple devices announce AirDrop over Bluetooth before they connect.

Receiving:

- On the other Apple device, open AirDrop, in the Finder or the share sheet.
  The T2 Mac shows up with its hostname after a few seconds.
- The T2 Mac only looks for senders while you are logged in and your screen
  is unlocked, as macOS does. Bluetooth listens for senders, Wi-Fi and AWDL
  only start once one is nearby and stop again shortly after it is gone.
- A sender that is not trusted yet raises a notification with the files and
  the buttons Accept, Always trust and Decline.

Sending:

- The receiving Apple device has to accept AirDrop from everyone, on macOS
  under AirDrop in the Finder, on iOS in Settings under General, AirDrop.
  Devices set to contacts only do not show up.
- If Bluetooth is off, the send window offers to turn it on.

Folders are not kept, the files of a received folder land next to each
other in the download folder. AirDrop is stopped before suspend and starts
again after resume.

## Quick Share

Quick Share is the sharing feature built into Android. Phone and Mac have to
be in the same Wi-Fi.

- On the phone, choose Share, then Quick Share, then the name of the Mac.
  The Mac shows up with its hostname.
- A phone that is not trusted yet raises a notification with the files, the
  PIN and the buttons Accept, Always trust and Decline. The PIN matches the
  one on the phone. Without an answer the transfer is declined after 60
  seconds.
- To send to a phone, Quick Share on it has to be visible to everyone and
  Bluetooth on the Mac has to be on, the Mac wakes the phone through
  Bluetooth. Accept on the phone, both sides show the same PIN. A phone set
  to contacts only appears as a hidden device.

## Trusted devices

Always trust accepts later transfers from that device without asking.
Neither Quick Share nor AirDrop gives a device a fixed identity outside of
the vendors' contact features, so T2 AirShare recognizes a trusted device by
its name only, for AirDrop and Quick Share alike. Give your phone or Mac a
unique name that is hard to guess, on Android under Settings, About phone,
Device name. Trusted devices are listed in the window and can be removed
there.

If that is not secure enough for you, leave T2 AirShare switched off or stop
the service with

```bash
systemctl --user disable --now t2-airshare.service
```

T2 AirShare was called T2 Share before AirDrop implementation. Updating
moves its settings and trusted devices over and switches the user service
to the new name.
