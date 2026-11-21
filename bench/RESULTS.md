# Memory and CPU: BatteryHub (Rust) v1.0 vs. original BatteryHub (C++) 1.1.0

Measured on 2026-11-21 on Windows 11 (build 26300), x64, with the same devices connected: Logitech PRO 2 LIGHTSPEED, SteelSeries Arctis Nova 3P receiver and AirPods (Bluetooth). Both programs used their defaults: taskbar strip, battery refresh every minute.

Each program was sampled every 2 s with [`measure.ps1`](measure.ps1), which reports the numbers Task Manager shows. "Steady" means after start-up (t ≥ 30 s).

## Result

| | **Rust v1.0** | C++ 1.1.0 | Difference |
|---|---:|---:|---:|
| Program file size | **368 KB** | 1,958 KB | −81% |
| Working set (avg, steady) | **2.4 MB** | 29.4 MB | −92% |
| Working set during a read (peak) | **17.8 MB** | 29.8 MB | −40% |
| Private memory (avg, steady) | **3.2 MB** | 5.6 MB | −43% |
| Handles (max) | **229** | 392 | −42% |
| Threads (max) | **10** | 15 | −33% |
| CPU time per minute (steady) | **~0.02 s** | ~0.64 s | −97% |

## What the optimisation changed (Rust, before → after)

Both builds were measured back to back under the same conditions (3 minutes each, the C++ app running at the same time in both runs): [`baseline.csv`](baseline.csv) and [`optimized.csv`](optimized.csv).

| | Before | After |
|---|---:|---:|
| Working set (avg, steady) | 23.7 MB | **2.4 MB** |
| Private memory (avg, steady) | 3.9 MB | **3.2 MB** |
| Handles (max) | 303 | **229** |
| CPU time per minute | 0.37 s | **0.02 s** |

What was changed:

1. **Bluetooth LE listening only while reading.** The AirPods / Fast Pair broadcast watcher used to run all the time and handle every advertisement nearby (dozens per second). It now runs for the few seconds of each read and is stopped in between. This is the main CPU saving.
2. **No full device scan on every read.** The HID device list (which opens every HID device) is cached. It is rebuilt only at start-up, after a device connects or disconnects, on *Refresh devices*, and at least every 10 minutes. Container ids of devices are cached too.
3. **Bluetooth fallback reads only Bluetooth device nodes** (`BTHENUM`, `BTHLE`, `BTHLEDEVICE`) instead of enumerating every device node on the system.
4. **Windows.Gaming.Input is loaded only when an Xbox-type controller is present** (an XInput slot in use, or a Microsoft / GameSir gamepad over Bluetooth).
5. **No IME / text services**, since the program never takes text input.
6. **Memory used only during a read is handed back** to Windows after each read (`SetProcessWorkingSetSize`). This is why the steady working set is low. During a read it briefly rises again (peak 17.8 MB), mostly from shared system DLLs.

## Notes

- Working set includes shared system DLLs (GDI, WinRT, Bluetooth, and DLLs that tools such as Windhawk inject into every process). Private memory is what the program itself allocates.
- The C++ app does more: about 100 device drivers, battery history, an updater, brand colours and more options. Part of the difference is less code doing less work, not only the language.
- The C++ app restarted itself about 40 s after launch, so its steady state was measured on the restarted process. Its first-run samples are in [`cpp.csv`](cpp.csv). The first Rust run (before the optimisation) is in [`rust.csv`](rust.csv).

Reproduce:

```powershell
.\bench\measure.ps1 -Exe "$env:LOCALAPPDATA\Programs\BatteryHub\Taskbar\batteryhub-taskbar.exe" -Seconds 180 -Csv bench\optimized.csv
.\bench\measure.ps1 -Exe "C:\Program Files\BatteryHub\BatteryHub.exe" -Seconds 180 -Csv bench\cpp.csv
```
