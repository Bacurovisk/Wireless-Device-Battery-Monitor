//! ASUS ROG Strix Impact II Wireless mouse over its 2.4 GHz receiver (VID 0B05, PID 1949).
//!
//! Protocol as used by G-Helper (GPL-3.0, github.com/seerge/g-helper, app/Peripherals/Mouse/AsusMouse.cs
//! and Models/StrixImpactIIWireless.cs, added by an owner of the mouse); implemented here from those
//! facts. Vendor collection on interface 0, 65-byte output / input report 00:
//!
//!   request  00 12 07 00 ...       battery
//!   reply    00 12 07 .. .. [5] .. .. .. .. [10]
//!            [5] = level in steps 0..4 (x 25 %), [10] != 0 = charging
//!
//! A level of 0 without charging means the mouse is off or asleep (the receiver answers for it).
//! Only this read is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x0B05;
const PID: u16 = 0x1949;
const REPLY_MS: u64 = 600;

pub fn request() -> [u8; 3] {
    [0x00, 0x12, 0x07]
}

/// Some((level, charging)) from a reply (report id included); Some(None) for "asleep".
pub fn decode(r: &[u8]) -> Option<Option<(i32, bool)>> {
    if r.len() < 11 || r[..3] != request() || r[5] > 4 {
        return None;
    }
    let charging = r[10] != 0;
    Some((r[5] > 0 || charging).then_some((r[5] as i32 * 25, charging)))
}

fn read(c: &HidColl) -> Option<Option<(i32, bool)>> {
    let d = HidDev::open(c)?;
    d.flush();
    if !d.write(&request()) {
        return None;
    }
    let end = Deadline::in_ms(REPLY_MS);
    while !end.passed() {
        if let Some(v) = d.read(end.left_ms()).and_then(|r| decode(&r)) {
            return Some(v);
        }
    }
    Some(None)
}

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.iface == 0 && c.page() >= 0xFF00 && c.in_len() == 65 && c.out_len() == 65);
    let Some(found) = colls.iter().find_map(read) else { return };
    let base = DeviceStatus {
        key: "asus:1949".into(),
        name: "ROG Strix Impact II Wireless".into(),
        kind: DeviceKind::Mouse,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 300,
        ..Default::default()
    };
    let s = match found {
        Some((level, charging)) => DeviceStatus { percent: level, charging, ..base },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(level: u8, charging: u8) -> Vec<u8> {
        let mut r = vec![0u8; 65];
        r[..3].copy_from_slice(&[0x00, 0x12, 0x07]);
        r[5] = level;
        r[10] = charging;
        r
    }

    #[test]
    fn replies() {
        assert_eq!(decode(&reply(3, 0)), Some(Some((75, false))));
        assert_eq!(decode(&reply(2, 1)), Some(Some((50, true))));
        assert_eq!(decode(&reply(0, 0)), Some(None)); // asleep
        assert_eq!(decode(&reply(9, 0)), None);
        let mut e = reply(3, 0);
        e[1] = 0xFF; // error reply
        assert_eq!(decode(&e), None);
    }
}
