//! FNATIC BOLT 2.4 GHz receiver (2F0E:0203) and USB cable (2F0E:0204).
//!
//! Hardware evidence: USBPcap captures and the FNATIC OP battery display.
//! Unnumbered HID output: 0C 00 ...; input: 0C PP ... (PP = battery %).
//! CAUTION: The charging flag below is *inferred* from a wired connection and
//! battery below 100%, not decoded from a device charging-status field.
//! FNATIC OP traces showed a transient wired 100% before the actual 65%.
//! A first wired 100% is held for 15s and may use the last known non-full
//! value while waiting; repeated values at the same instant do not confirm it.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::HidDev;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const VID: u16 = 0x2F0E;
const RECEIVER: u16 = 0x0203;
const CABLE: u16 = 0x0204;
const BATTERY_COMMAND: u8 = 0x0C;
const TIMEOUT_MS: u32 = 1200;
const WIRED_FULL_CONFIRM: Duration = Duration::from_secs(15);

#[derive(Default)]
struct WiredFullFilter {
    first_full: Option<Instant>,
    last_non_full: Option<i32>,
}

impl WiredFullFilter {
    /// Reject a newly encountered wired 100% until it persists for 15s.
    /// USB receiver captures observed a bogus 100% for about eight seconds.
    fn filter(&mut self, wired: Option<i32>, wireless: Option<i32>, now: Instant) -> Option<i32> {
        match wired {
            Some(0..=99) => {
                self.first_full = None;
                self.last_non_full = wired;
                wired
            }
            Some(100) => {
                let since = *self.first_full.get_or_insert(now);
                if now.saturating_duration_since(since) >= WIRED_FULL_CONFIRM {
                    self.last_non_full = None;
                    Some(100)
                } else {
                    let observed = wireless.filter(|p| (0..100).contains(p));
                    if observed.is_some() { self.last_non_full = observed; }
                    observed.or(self.last_non_full)
                }
            }
            _ => {
                self.first_full = None;
                None
            }
        }
    }
    fn wireless_only(&mut self, percent: Option<i32>) {
        self.first_full = None;
        if let Some(p @ 0..=99) = percent { self.last_non_full = Some(p); }
        if percent == Some(100) { self.last_non_full = None; }
    }
}

static FULL_FILTER: OnceLock<Mutex<WiredFullFilter>> = OnceLock::new();
fn full_filter() -> &'static Mutex<WiredFullFilter> {
    FULL_FILTER.get_or_init(|| Mutex::new(WiredFullFilter::default()))
}


/// Input report payload after HidDev strips a possible zero report ID.
pub fn decode(payload: &[u8]) -> Option<i32> {
    if payload.len() == 64 && payload[0] == BATTERY_COMMAND && payload[1] <= 100 {
        Some(payload[1] as i32)
    } else {
        None
    }
}

fn query(d: &HidDev) -> Option<i32> {
    d.flush();
    if !d.write(&[0x00, BATTERY_COMMAND]) {
        return None;
    }
    let deadline = crate::util::Deadline::in_ms(TIMEOUT_MS as u64);
    loop {
        let left = deadline.left_ms();
        if left == 0 {
            return None;
        }
        // A timed-out read does not imply the mouse is disconnected.
        let data = d.read(left)?;
        if let Some(percent) = decode(&data) {
            return Some(percent);
        }
    }
}

fn read_device(ctx: &PollContext, pid: u16) -> Option<i32> {
    // The receiver uses MI_01, usage page FF (from hardware HID dump).
    // Cable MI_01 is observed in Windows PnP; the report layout must be
    // verified on wired hardware as well.
    let colls = ctx.find(VID, pid, |c| {
        c.iface == 1 && c.page() == 0x00FF && c.usage() == 0x0001
            && c.in_len() == 65 && c.out_len() == 65
    });
    colls.iter().find_map(|c| HidDev::open(c).and_then(|d| query(&d)))
}

pub fn poll(ctx: &mut PollContext) {
    let cable_present = !ctx.find(VID, CABLE, |_| true).is_empty();
    let dongle_present = !ctx.find(VID, RECEIVER, |_| true).is_empty();
    if !cable_present && !dongle_present {
        return;
    }
    // Only touch unclaimed devices; the driver is called before generic HID.
    if (cable_present && ctx.is_claimed(VID, CABLE))
        || (dongle_present && ctx.is_claimed(VID, RECEIVER)) {
        return;
    }

    let wired = if cable_present { read_device(ctx, CABLE) } else { None };
    // When a wired 100% first appears, also query the dongle. This gives a
    // useful fallback instead of showing a known transient full reading.
    let wireless = if dongle_present && (wired.is_none() || wired == Some(100)) {
        read_device(ctx, RECEIVER)
    } else {
        None
    };
    let mut state = full_filter().lock().unwrap_or_else(|e| e.into_inner());
    let chosen = if cable_present {
        match wired {
            Some(_) => Some((CABLE, state.filter(wired, wireless, Instant::now()))),
            None => {
                state.wireless_only(wireless);
                wireless.map(|p| (RECEIVER, Some(p)))
            }
        }
    } else {
        state.wireless_only(wireless);
        wireless.map(|p| (RECEIVER, Some(p)))
    };
    drop(state);
    let selected_pid = chosen.map(|v| v.0).unwrap_or(if cable_present { CABLE } else { RECEIVER });
    let is_wired = selected_pid == CABLE;
    let mut status = DeviceStatus {
        key: "fnatic:bolt".into(),
        name: "FNATIC BOLT".into(),
        kind: DeviceKind::Mouse,
        via: if is_wired { "USB cable" } else { "2.4 GHz receiver" }.into(),
        hw: Some((VID, selected_pid)),
        grace_sec: 60,
        ..Default::default()
    };
    if let Some((_, percent)) = chosen {
        // A temporarily suppressed wired 100% deliberately has unknown percent
        // if neither the receiver nor a previous valid level is available.
        if let Some(percent) = percent {
            status.percent = percent;
            status.charging = inferred_charging(is_wired, percent);
        }
        if is_wired {
            status.detail = "Charging inferred from USB cable; initial 100% filtered".into();
        }
    } else {
        status.connected = false;
    }
    ctx.add(status, VID, selected_pid);
    // Both USB identities can be present at once. Never show duplicates.
    if cable_present { ctx.claim(VID, CABLE); }
    if dongle_present { ctx.claim(VID, RECEIVER); }
}

fn inferred_charging(wired: bool, percent: i32) -> bool {
    wired && (0..100).contains(&percent)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recorded_readings() {
        for (hex, expected) in [(0x2C,44),(0x41,65),(0x3F,63),(0x64,100)] {
            let mut report = [0u8;64];
            report[0] = 0x0C;
            report[1] = hex;
            assert_eq!(decode(&report),Some(expected));
        }
    }
    #[test]
    fn invalid_reports_rejected() {
        assert_eq!(decode(&[0x0C,44]),None);
        let mut r = [0u8;64];
        r[0]=0x0D; r[1]=44;
        assert_eq!(decode(&r),None);
        r[0]=0x0C; r[1]=101;
        assert_eq!(decode(&r),None);
    }
    #[test]
    fn initial_wired_full_does_not_flash_100() {
        let t = Instant::now();
        let mut s = WiredFullFilter::default();
        s.wireless_only(Some(65));
        assert_eq!(s.filter(Some(100), Some(65), t), Some(65));
        assert_eq!(s.filter(Some(100), None, t + Duration::from_secs(9)), Some(65));
        assert_eq!(s.filter(Some(65), None, t + Duration::from_secs(10)), Some(65));
        assert_eq!(s.first_full, None);
    }
    #[test]
    fn actual_full_is_accepted_after_confirmation() {
        let t = Instant::now();
        let mut s = WiredFullFilter::default();
        assert_eq!(s.filter(Some(100), None, t), None);
        assert_eq!(s.filter(Some(100), None, t + Duration::from_secs(14)), None);
        assert_eq!(s.filter(Some(100), None, t + Duration::from_secs(15)), Some(100));
        assert_eq!(s.filter(Some(88), None, t + Duration::from_secs(16)), Some(88));
    }
    #[test]
    fn cable_reconnect_starts_new_confirmation_window() {
        let t = Instant::now();
        let mut s = WiredFullFilter::default();
        assert_eq!(s.filter(Some(100), None, t), None);
        s.wireless_only(Some(64));
        assert_eq!(s.filter(Some(100), None, t + Duration::from_secs(20)), Some(64));
    }
    #[test]
    fn charge_is_only_an_inference() {
        assert!(inferred_charging(true,65));
        assert!(!inferred_charging(false,65));
        assert!(!inferred_charging(true,100));
    }
}
