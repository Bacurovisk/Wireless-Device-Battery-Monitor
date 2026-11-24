//! Per-device settings and the device menus. Each edition keeps them under its settings key, one
//! value per device key (stable across reconnects and restarts):
//!  * `Thresholds`: red warning level (DWORD, percent);
//!  * `Names`: name chosen with "Rename device" (REG_SZ; the device itself is not renamed);
//!  * `Warn5`: "Warn at 5%" (DWORD 0/1, default on).
use std::collections::{HashMap, HashSet};

use windows::Win32::UI::WindowsAndMessaging::{AppendMenuW, CreatePopupMenu, HMENU, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED};

use crate::device::DeviceStatus;
use crate::install::{self as inst, HKCU};
use crate::ui::{MenuIcons, glyph};

/// The levels offered in the menus.
pub const THRESHOLDS: [i32; 7] = [50, 40, 30, 20, 15, 10, 5];
pub const DEFAULT_THRESHOLD: i32 = 15;
/// "Warn at 5%": shown once when the level falls to this, armed again at REARM_LEVEL or on power.
pub const CRITICAL_LEVEL: i32 = 5;
pub const REARM_LEVEL: i32 = 10;
/// Longest name accepted by "Rename device".
pub const NAME_MAX: usize = 48;

fn sub(settings_key: &str, name: &str) -> String {
    format!(r"{settings_key}\{name}")
}

/// Warning level for one device (the percentage at or below which it turns red).
pub fn threshold(settings_key: &str, device_key: &str) -> i32 {
    inst::open(HKCU, &sub(settings_key, "Thresholds"), false)
        .and_then(|k| k.get(device_key))
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .map(|v| v as i32)
        .filter(|v| THRESHOLDS.contains(v))
        .unwrap_or(DEFAULT_THRESHOLD)
}

pub fn set_threshold(settings_key: &str, device_key: &str, value: i32) -> bool {
    inst::create(HKCU, &sub(settings_key, "Thresholds")).is_some_and(|k| k.set_dword(device_key, value as u32))
}

/// Low = discharging, a known level, at or below the device's warning level.
pub fn is_low(s: &DeviceStatus, threshold: i32) -> bool {
    !s.charging && !s.is_full() && s.connected && s.percent >= 0 && s.percent <= threshold
}

// ---- names -------------------------------------------------------------------------------------

/// A name typed by the user, cleaned up: no control characters, single spaces, at most NAME_MAX
/// characters. None when nothing is left (the device's own name is used then).
pub fn clean_name(s: &str) -> Option<String> {
    let s: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.chars().take(NAME_MAX).collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Every name chosen with "Rename device": device key -> name.
pub fn names(settings_key: &str) -> HashMap<String, String> {
    inst::open(HKCU, &sub(settings_key, "Names"), false)
        .map(|k| k.values().into_iter().filter(|v| v.1 == 1).filter_map(|(n, _, d)| clean_name(&d).map(|d| (n, d))).collect())
        .unwrap_or_default()
}

/// Saves a name (None or an empty name goes back to the device's own name).
pub fn set_name(settings_key: &str, device_key: &str, name: Option<&str>) -> bool {
    match name.and_then(clean_name) {
        Some(n) => inst::create(HKCU, &sub(settings_key, "Names")).is_some_and(|k| k.set_string(device_key, &n)),
        None => inst::open(HKCU, &sub(settings_key, "Names"), true).is_none_or(|k| {
            k.delete_value(device_key);
            true
        }),
    }
}

// ---- "Warn at 5%" ------------------------------------------------------------------------------

pub fn warn_critical(settings_key: &str, device_key: &str) -> bool {
    inst::open(HKCU, &sub(settings_key, "Warn5"), false).and_then(|k| k.get(device_key)).is_none_or(|v| v != "0x00000000")
}

pub fn set_warn_critical(settings_key: &str, device_key: &str, on: bool) -> bool {
    inst::create(HKCU, &sub(settings_key, "Warn5")).is_some_and(|k| k.set_dword(device_key, on as u32))
}

/// Which devices were already warned about: one warning when the level falls to 5% or below,
/// none on later readings, and a new one only after the device charged (on power, or back at
/// REARM_LEVEL or above).
#[derive(Default, Debug)]
pub struct CriticalWarnings {
    warned: HashSet<String>,
}

impl CriticalWarnings {
    /// Returns true when a warning should be shown for this reading now.
    pub fn check(&mut self, key: &str, level: i32, on_power: bool) -> bool {
        if level < 0 {
            return false; // unknown level: neither warn nor re-arm
        }
        if on_power || level >= REARM_LEVEL {
            self.warned.remove(key);
            return false;
        }
        level <= CRITICAL_LEVEL && self.warned.insert(key.to_string())
    }
}

// ---- menus -------------------------------------------------------------------------------------

/// Menu ids: BASE + device index * STRIDE + one of the item offsets below.
pub const MENU_BASE: usize = 1000;
const STRIDE: usize = 32;
const ITEM_DASHBOARD: usize = 0;
const ITEM_HISTORY: usize = 1;
const ITEM_STATS: usize = 2;
const ITEM_RENAME: usize = 3;
const ITEM_RESET_NAME: usize = 4;
const ITEM_WARN5: usize = 5;
const ITEM_THRESHOLD: usize = 8; // + index into THRESHOLDS

/// What a device menu item asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeviceAction {
    /// Open the dashboard on a tab (0 overview, 1 history, 2 statistics).
    Dashboard(u8),
    Rename,
    ResetName,
    ToggleWarn5,
    Threshold(i32),
}

/// Appends one submenu per device ("Name (32%, on battery)"), in three sections: information
/// (dashboard, history, statistics), management (rename) and warnings (red warning level, warn
/// at 5%). Every item has an icon. `renamed` holds the keys that have a name of the user's.
pub fn append_device_menus(menu: HMENU, devices: &[DeviceStatus], settings_key: &str, renamed: &HashSet<String>, icons: &mut MenuIcons) {
    unsafe {
        for (i, d) in devices.iter().enumerate() {
            let Ok(sub) = CreatePopupMenu() else { continue };
            MenuIcons::check_or_icon(sub);
            let id = |item: usize| MENU_BASE + i * STRIDE + item;
            let _ = AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, &hs(&format!("{} - {}", d.name, d.charge_text())));
            let _ = AppendMenuW(sub, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
            for (item, label, g) in [
                (ITEM_DASHBOARD, "Dashboard...", glyph::DASHBOARD),
                (ITEM_HISTORY, "Battery history", glyph::HISTORY),
                (ITEM_STATS, "Statistics", glyph::STATISTICS),
            ] {
                let _ = AppendMenuW(sub, MF_STRING, id(item), &hs(label));
                icons.set(sub, id(item) as u32, false, g);
            }
            let _ = AppendMenuW(sub, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
            let _ = AppendMenuW(sub, MF_STRING, id(ITEM_RENAME), &hs("Rename device..."));
            icons.set(sub, id(ITEM_RENAME) as u32, false, glyph::RENAME);
            if renamed.contains(&d.key) {
                let _ = AppendMenuW(sub, MF_STRING, id(ITEM_RESET_NAME), &hs("Use the device's own name"));
                icons.set(sub, id(ITEM_RESET_NAME) as u32, false, glyph::RESET);
            }
            let _ = AppendMenuW(sub, MF_SEPARATOR, 0, windows::core::PCWSTR::null());
            // Red warning level.
            if let Ok(levels) = CreatePopupMenu() {
                let current = threshold(settings_key, &d.key);
                for (j, t) in THRESHOLDS.iter().enumerate() {
                    let check = if *t == current { MF_CHECKED } else { MF_UNCHECKED };
                    let def = if *t == DEFAULT_THRESHOLD { "  (default)" } else { "" };
                    let _ = AppendMenuW(levels, MF_STRING | check, id(ITEM_THRESHOLD + j), &hs(&format!("{t}%{def}")));
                }
                let n = windows::Win32::UI::WindowsAndMessaging::GetMenuItemCount(Some(sub));
                let _ = AppendMenuW(sub, MF_STRING | MF_POPUP, levels.0 as usize, &hs(&format!("Red warning at or below {current}%")));
                icons.set(sub, n as u32, true, glyph::WARNING);
            }
            let check = if warn_critical(settings_key, &d.key) { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(sub, MF_STRING | check, id(ITEM_WARN5), &hs("Warn at 5% (notification)"));
            icons.set(sub, id(ITEM_WARN5) as u32, false, glyph::BELL);

            let n = windows::Win32::UI::WindowsAndMessaging::GetMenuItemCount(Some(menu));
            let label = format!("{}  ({})", d.name, d.charge_text());
            let _ = AppendMenuW(menu, MF_STRING | MF_POPUP, sub.0 as usize, &hs(&label));
            icons.set(menu, n as u32, true, d.kind.glyph());
        }
    }
}

/// Decodes a menu id from `append_device_menus` into (device index, action).
pub fn decode_menu_id(id: usize) -> Option<(usize, DeviceAction)> {
    let rel = id.checked_sub(MENU_BASE)?;
    let (dev, item) = (rel / STRIDE, rel % STRIDE);
    let action = match item {
        ITEM_DASHBOARD => DeviceAction::Dashboard(0),
        ITEM_HISTORY => DeviceAction::Dashboard(1),
        ITEM_STATS => DeviceAction::Dashboard(2),
        ITEM_RENAME => DeviceAction::Rename,
        ITEM_RESET_NAME => DeviceAction::ResetName,
        ITEM_WARN5 => DeviceAction::ToggleWarn5,
        i if i >= ITEM_THRESHOLD => DeviceAction::Threshold(*THRESHOLDS.get(i - ITEM_THRESHOLD)?),
        _ => return None,
    };
    Some((dev, action))
}

fn hs(s: &str) -> windows::core::HSTRING {
    windows::core::HSTRING::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_round_trip() {
        for dev in 0..5 {
            for (j, t) in THRESHOLDS.iter().enumerate() {
                assert_eq!(decode_menu_id(MENU_BASE + dev * STRIDE + ITEM_THRESHOLD + j), Some((dev, DeviceAction::Threshold(*t))));
            }
            assert_eq!(decode_menu_id(MENU_BASE + dev * STRIDE + ITEM_RENAME), Some((dev, DeviceAction::Rename)));
            assert_eq!(decode_menu_id(MENU_BASE + dev * STRIDE + ITEM_STATS), Some((dev, DeviceAction::Dashboard(2))));
        }
        assert_eq!(decode_menu_id(3), None);
        assert_eq!(decode_menu_id(MENU_BASE + 6), None);
    }

    #[test]
    fn critical_warning_once_and_rearmed_after_charging() {
        let mut w = CriticalWarnings::default();
        assert!(!w.check("m", 12, false));
        assert!(!w.check("m", 6, false));
        assert!(w.check("m", 5, false)); // reached 5%: warn
        assert!(!w.check("m", 5, false)); // same level again: no repeat
        assert!(!w.check("m", 3, false)); // falling further: still once
        assert!(!w.check("m", 7, false)); // a bounce under 10% does not re-arm
        assert!(!w.check("m", 4, false));
        assert!(!w.check("m", -1, false)); // unknown level changes nothing
        assert!(!w.check("m", 4, true)); // plugged in: re-armed, no warning while on power
        assert!(w.check("m", 5, false)); // unplugged and still low: warned again
        assert!(!w.check("k", 9, false));
        assert!(w.check("k", 1, false)); // first reading already below 5%
        assert!(!w.check("k", 50, false));
        assert!(w.check("k", 5, false));
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  My   mouse \n"), Some("My mouse".into()));
        assert_eq!(clean_name("\t\r\n "), None);
        assert_eq!(clean_name(&"x".repeat(100)).unwrap().chars().count(), NAME_MAX);
        assert_eq!(clean_name("Słuchawki 🎧"), Some("Słuchawki 🎧".into()));
    }

    #[test]
    fn names_survive_in_the_registry() {
        let sk = r"Software\BatteryHub-Test-Names";
        assert!(set_name(sk, "logi:unit:ABC", Some("Office mouse")));
        assert_eq!(names(sk).get("logi:unit:ABC").map(String::as_str), Some("Office mouse"));
        assert!(set_name(sk, "logi:unit:ABC", None));
        assert!(!names(sk).contains_key("logi:unit:ABC"));
        assert!(warn_critical(sk, "x")); // default on
        assert!(set_warn_critical(sk, "x", false));
        assert!(!warn_critical(sk, "x"));
        assert!(inst::delete_tree(HKCU, sk));
    }
}
