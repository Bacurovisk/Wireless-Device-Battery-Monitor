# BatteryHub v1.1 (Rust)

An ultra-lightweight Rust implementation of BatteryHub for Windows, a fork of [krisgfx9/Wireless-Device-Battery-Monitor](https://github.com/krisgfx9/Wireless-Device-Battery-Monitor). All credit for the original idea, look and device protocols goes to the original project.

## Download

| File | |
|---|---|
| `BatteryHub-Taskbar-Setup-v1.1.exe` | Setup: Install / Update / Uninstall |
| `SHA256SUMS.txt` | Checksum |

Already installed? Run the new setup and choose **[2] Update**. Your settings and autostart choice are kept. From v1.1 on, you can also update from the program itself.

## New

- **Check for updates** (right-click menu): asks GitHub for the latest release and shows "No new updates" or "A new update is available: v1.1 -> v1.x - update now?". On *Yes* the setup is downloaded, verified against the release's SHA-256 checksum and started; it updates and restarts BatteryHub.
- **About 0.2 MB of RAM when idle** (Task Manager "Memory" column; was about 1.1 MB): memory that is only needed for a moment is handed back to Windows after every read and Bluetooth check, and settings are cached instead of read from the registry every second.
- **Setup:** when it is done (install, update, uninstall or exit), it thanks you, links to the project for a star and closes itself after 5 seconds. On an error it stays open.

## Fixes since v1.0 (v1.0.1 / v1.0.1.1)

- Unplugged receiver / cable: its device disappears at once instead of showing "asleep".
- Bluetooth devices (AirPods...) disappear / appear right when they disconnect / connect; AirPods only while connected to this PC.
- Headsets switched on or off on a 2.4 GHz dongle, and mice waking up behind a receiver, show within ~10 s.
- No duplicate, garbled strip after an Explorer restart (seen on the small Windows 10 taskbar of ExplorerPatcher).
- Safe fallback to notification-area icons on a vertical taskbar or an unknown Explorer layout; unit tests and CI.

Windows may show a SmartScreen warning because the file is not code-signed: click *More info* -> *Run anyway*.
