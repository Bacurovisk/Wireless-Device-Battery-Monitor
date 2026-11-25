//! Audeze Maxwell and Maxwell 2 headsets over their 2.4 GHz dongles (VID 3329).
//!
//! Protocol from HeadsetControl (GPL-3.0, github.com/Sapd/HeadsetControl, lib/devices/
//! audeze_maxwell_device.hpp, which lists the Maxwell 2's 4B29 / 4B28) and HaloBattery (MIT,
//! github.com/HeyOkay/HaloBattery, providers/audeze.py, measured on a Maxwell dongle 4B19);
//! implemented here from those facts. Vendor collection FF13:0001, 62-byte reports:
//!
//!   request  06 07 80 05 5A 03 00 D6 0C 00 ..      output report 06, attribute 0CD6 = battery
//!   reply    07 .. D6 0C 00 00 <percent> ..        input report 07; it is a rolling buffer of the
//!                                                  dongle's last replies, so it is searched
//!
//! No charging flag. With the headset off the dongle keeps answering its last value, so it is only
//! asked while its product string says a headset is linked ("Audeze Maxwell HID", against "Audeze
//! Maxwell Dongle" alone). Only this read is sent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{self, HidColl, HidDev};
use crate::util::{Deadline, hex4};

const VID: u16 = 0x3329;
const MODELS: [(u16, &str); 4] = [
    (0x4B19, "Audeze Maxwell"),
    (0x4B18, "Audeze Maxwell (Xbox)"),
    (0x4B29, "Audeze Maxwell 2"),
    (0x4B28, "Audeze Maxwell 2 (Xbox)"),
];
const REQUEST: [u8; 9] = [0x06, 0x07, 0x80, 0x05, 0x5A, 0x03, 0x00, 0xD6, 0x0C];
const MARKER: [u8; 4] = [0xD6, 0x0C, 0x00, 0x00];
const REPLY_MS: u64 = 600;

/// The level after the first plausible battery marker in an input report.
pub fn find_level(r: &[u8]) -> Option<i32> {
    r.windows(5).find(|w| w[..4] == MARKER && w[4] <= 100).map(|w| w[4] as i32)
}

fn read(c: &HidColl) -> Option<i32> {
    let d = HidDev::open(c)?;
    d.flush();
    if !d.write(&REQUEST) {
        return None;
    }
    let end = Deadline::in_ms(REPLY_MS);
    while !end.passed() {
        if let Some(level) = d.read(end.left_ms()).filter(|r| r.first() == Some(&0x07)).and_then(|r| find_level(&r)) {
            return Some(level);
        }
    }
    None
}

pub fn poll(ctx: &mut PollContext) {
    for (pid, name) in MODELS {
        if ctx.is_claimed(VID, pid) {
            continue;
        }
        let colls = ctx.find(VID, pid, |c| c.page() == 0xFF13 && c.usage() == 0x0001 && c.in_len() >= 16 && c.out_len() >= 16);
        let Some(c) = colls.first() else { continue };
        let base = DeviceStatus {
            key: format!("audeze:{}", hex4(pid)),
            name: name.into(),
            kind: DeviceKind::Headset,
            via: "2.4 GHz dongle".into(),
            hw: Some((VID, pid)),
            grace_sec: 60,
            ..Default::default()
        };
        // "… Dongle": no headset linked (the dongle would answer its last value).
        let linked = !hid::product_string(c).to_lowercase().contains("dongle");
        let s = match linked.then(|| colls.iter().find_map(read)).flatten() {
            Some(level) => DeviceStatus { percent: level, ..base },
            None => DeviceStatus { connected: false, ..base },
        };
        ctx.add(s, VID, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        let mut r = vec![0u8; 62];
        r[0] = 0x07;
        r[20..25].copy_from_slice(&[0xD6, 0x0C, 0x00, 0x00, 0xC8]); // an old frame, not a level
        r[40..45].copy_from_slice(&[0xD6, 0x0C, 0x00, 0x00, 76]);
        assert_eq!(find_level(&r), Some(76));
        assert_eq!(find_level(&[0x07, 0xD6, 0x0C, 0x00]), None);
    }
}
