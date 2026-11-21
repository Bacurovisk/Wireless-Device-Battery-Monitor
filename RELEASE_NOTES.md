# BatteryHub v1.0.1 (Rust)

An ultra-lightweight Rust implementation of BatteryHub for Windows, a fork of [krisgfx9/Wireless-Device-Battery-Monitor](https://github.com/krisgfx9/Wireless-Device-Battery-Monitor). All credit for the original idea, look and device protocols goes to the original project.

## Download

| File | |
|---|---|
| `BatteryHub-Taskbar-Setup-v1.0.1.exe` | Setup: Install / Update / Uninstall |
| `SHA256SUMS.txt` | Checksum |

Already on v1.0? Run the new setup and choose **[2] Update**. Your warning levels, refresh options and autostart choice are kept.

## Fixes

- **Unplugging a receiver or cable** now removes its device at once, instead of showing it as "asleep". "Asleep" is used only when the receiver is still plugged in and the device sleeps.
- **Bluetooth devices disappear right after they disconnect**, and appear right after they connect. Bluetooth connection changes are now detected (checked every 4 s), and HID devices register for Windows' arrival / removal notifications.
- **AirPods are shown only while connected to this PC**, not while they are just nearby or connected to a phone.
- **Strip drawing after leaving full screen, DPI / theme changes and Explorer restarts:** the strip is re-rendered when the taskbar changes and recreated reliably after Explorer restarts.

## New

- **Safe fallback:** on a vertical taskbar or an unknown Explorer layout, BatteryHub shows one notification-area icon per device (level drawn in the icon, same tooltip and menu) and returns to the strip when it can.
- **Tests and CI:** unit tests decode recorded reports (SteelSeries, DualSense / DualShock 4, AirPods, Fast Pair, Logitech) without the hardware; GitHub Actions builds and tests every push.

## Footprint

RAM as Task Manager shows it ("Memory"): **0.8 MB** vs 4.2 MB for the original C++ app on the same PC. Details are in `bench/RESULTS.md`.

Windows may show a SmartScreen warning because the file is not code-signed: click *More info* → *Run anyway*.
