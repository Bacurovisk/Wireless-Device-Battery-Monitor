# BatteryHub v1.0.1.1 (Rust)

An ultra-lightweight Rust implementation of BatteryHub for Windows, a fork of [krisgfx9/Wireless-Device-Battery-Monitor](https://github.com/krisgfx9/Wireless-Device-Battery-Monitor). All credit for the original idea, look and device protocols goes to the original project.

## Download

| File | |
|---|---|
| `BatteryHub-Taskbar-Setup-v1.0.1.1.exe` | Setup: Install / Update / Uninstall |
| `SHA256SUMS.txt` | Checksum |

Already installed? Run the new setup and choose **[2] Update**. Your settings and autostart choice are kept.

## Fixes

- **Garbled strip after an Explorer restart:** a second, orphaned strip could be left on the taskbar, overlapping the real one and the notification area. Now there is always exactly one strip, and any stray strip window is removed within a second.
- No notification-area icons flashing up for a moment when the taskbar is not ready yet at sign-in.
- The setup now also reads a four-part version (e.g. 1.0.1.1) from the installed program.

## Changes since v1.0 (from v1.0.1)

- Unplugged receiver / cable: its device disappears at once instead of showing "asleep".
- Bluetooth devices disappear / appear right when they disconnect / connect. AirPods are shown only while connected to this PC.
- Headsets switched on or off on a 2.4 GHz dongle, and mice waking up behind a receiver, show within ~10 s.
- A device change during a running read is no longer lost.
- Strip re-rendered after full screen and DPI / theme changes; safe fallback to notification-area icons on a vertical taskbar or an unknown Explorer layout.
- Unit tests for recorded device reports, and CI.

RAM as Task Manager shows it ("Memory"): **0.8 MB** vs 4.2 MB for the original C++ app on the same PC (`bench/RESULTS.md`).

Windows may show a SmartScreen warning because the file is not code-signed: click *More info* → *Run anyway*.
