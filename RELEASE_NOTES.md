# BatteryHub v1.0 (Rust)

First release of this fork of [krisgfx9/Wireless-Device-Battery-Monitor](https://github.com/krisgfx9/Wireless-Device-Battery-Monitor), rewritten in **Rust** 🦀. All credit for the original idea, look and device protocols goes to the original project.

## Download

| File | |
|---|---|
| `BatteryHub-Taskbar-Setup-v1.0.exe` | Setup: Install / Update / Uninstall |
| `SHA256SUMS.txt` | Checksum |

Run the setup and choose **Install**, **Update** or **Uninstall**. Before showing the options, it checks what is already installed and shows where every file is. No administrator rights are needed. It does not touch the original C++ BatteryHub.

## What's included

- Taskbar strip with an icon and percentage per device. Hover a device for its name and charge state.
- Right click to:
  - set each device's red warning level (50 to 5%);
  - refresh devices or battery levels;
  - choose the refresh interval (1 / 5 / 10 / 30 min / 1 h);
  - turn on refresh when a device connects or disconnects;
  - exit.
- Green ⚡ while charging, blue ⚡ at 100%, red percentage at or below the warning level.
- Desktop and Start menu shortcuts, optional autostart.
- Drivers: Logitech HID++, SteelSeries, Razer, DualShock 4 / DualSense, AirPods / Beats (BLE), Bluetooth (incl. Fast Pair), generic HID, Xbox.
- Light on resources: compared with the original 1.1.0 on the same PC, 368 KB vs 1,958 KB file, 2.4 MB vs 29.4 MB working set, 3.2 MB vs 5.6 MB private memory, ~0.02 s vs ~0.64 s CPU per minute. See `bench/RESULTS.md`.
- Tooltip and right-click menu follow the taskbar theme (dark / light).

Windows may show a SmartScreen warning because the file is not code-signed: click *More info* → *Run anyway*.
