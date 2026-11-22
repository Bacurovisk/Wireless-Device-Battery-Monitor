# BatteryHub v1.2.1 (Rust)

An ultra-lightweight Rust implementation of BatteryHub for Windows, a fork of [krisgfx9/Wireless-Device-Battery-Monitor](https://github.com/krisgfx9/Wireless-Device-Battery-Monitor). All credit for the original idea, look and device protocols goes to the original project.

## Download

| File | |
|---|---|
| `BatteryHub-Taskbar-Setup-v1.2.1.exe` | Setup: Install / Update / Uninstall |
| `SHA256SUMS.txt` | Checksum |

Already installed? Use *Check for updates...* in the right-click menu (v1.1 and later), or run the new setup and choose **[2] Update**. Your settings and autostart choice are kept.

## New in v1.2.1

- **Show widget on** (right-click menu, with more than one monitor), like the original: *All monitors*, *Main monitor only*, or *Display 2 (left) only* for any other monitor. The monitor is remembered by its identity; while it is unplugged, the widget is on the main taskbar.
- **Position on the taskbar:** next to the notification area or 40 / 80 / 120 / 160 / 200 px from it, to make room for other taskbar widgets (e.g. an iCUE sensor).
- **Background behind taskbar text (for translucent taskbars):** optional rounded plate in the taskbar's colour, off by default.

## New in v1.2

- **ROCCAT Syn Max Air** headset (1E7D:3A3F) over its 2.4 GHz receiver: level from the battery voltage, *Charging* / *Full* on the dock, disappears when switched off. Contributed by @BrooklynBanana in #1. ROCCAT Swarm must not run at the same time (it holds the receiver).
- **Request support for a device...** (right-click menu): lists every device BatteryHub can see (HID, USB, Bluetooth). Pick the one that is not shown, check its name and type, add notes, and *Send* opens a GitHub issue in your browser, filled in with exactly what the window shows (name, type, notes, vendor / product id, HID and USB interface summary, Windows build). You submit it there; nothing is sent from the app. *Copy full report* copies the same plus the list of all devices. Also `batteryhub-taskbar.exe --report`. Light or dark, following the Windows app theme.
- **Automatically start with Windows** (menu): switch the autostart entry on or off without the setup.
- **Automatically check for updates** (menu, off by default): checks once when BatteryHub starts and, if there is a new version, asks whether to update.
- **Shorter tooltip:** `PRO 2 LIGHTSPEED: 96%` (plus charging / full / asleep).
- **Setup:** the autostart line shows the registry entry (`HKCU\...\Run\BatteryHub Taskbar`) instead of the program path.

## v1.1

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
