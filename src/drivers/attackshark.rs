//! Attack Shark mice (X1 and others) on the Xenta 2.4 GHz receiver (VID 1D57, PID FA60).
//!
//! Protocol from Attack Shark Battery KDE by Gabriel Marques Ferrarezi (MIT,
//! gitlab.com/gabriel17166/Attack-Shark-Battery-KDE). The receiver's interface 2 pushes a 5-byte
//! input report about every 2 s on its own; nothing has to be sent:
//!
//!   03 E5 40 ST NN
//!   03     report ID
//!   ST     01 on battery, 03 charging
//!   NN     battery level 0..100 %
//!
//! `03 E5 01 00 00` frames show up while the radio link blinks and would read as 0 %; they are
//! skipped. No valid frame within the timeout means the mouse is switched off or asleep (the
//! receiver stays enumerated). With the cable plugged in, the mouse is a different USB device
//! (1D57:2155) and the receiver goes silent.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::HidDev;
use crate::util::hex4;

const VID: u16 = 0x1D57;
const RECEIVER: u16 = 0xFA60;
const ST_BATTERY: u8 = 0x01;
const ST_CHARGING: u8 = 0x03;
/// Frames come about every 2 s; wait a little longer than two of them.
const WAIT_MS: u64 = 4500;

/// Decodes one input report (report ID included, r[0] == 0x03).
pub fn decode(r: &[u8]) -> Option<(i32, bool)> {
    if r.len() < 5 || r[0] != 0x03 || r[1] != 0xE5 || r[2] != 0x40 {
        return None;
    }
    let level = r[4];
    if level > 100 || (r[3] != ST_BATTERY && r[3] != ST_CHARGING) {
        return None;
    }
    Some((level as i32, r[3] == ST_CHARGING))
}

fn base() -> DeviceStatus {
    DeviceStatus {
        key: format!("attackshark:{}", hex4(RECEIVER)),
        name: "Attack Shark mouse".into(),
        kind: DeviceKind::Mouse,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, RECEIVER)),
        grace_sec: 60,
        ..Default::default()
    }
}

fn wait_frame(d: &HidDev) -> Option<(i32, bool)> {
    let end = crate::util::Deadline::in_ms(WAIT_MS);
    loop {
        let left = end.left_ms();
        if left == 0 {
            return None;
        }
        if let Some(v) = d.read(left).and_then(|r| decode(&r)) {
            return Some(v);
        }
    }
}

pub fn poll(ctx: &mut PollContext) {
    for (vid, pid) in ctx.products(VID) {
        if pid != RECEIVER {
            continue;
        }
        // Interface 2, the collection whose input report is the 5-byte frame (page 000A on Windows).
        let colls = ctx.find(vid, pid, |c| c.iface == 2 && c.in_len() == 5);
        if colls.is_empty() {
            continue;
        }
        let mut found = None;
        for c in &colls {
            let Some(d) = HidDev::open_read_only(c) else { continue };
            // Some receivers answer GET_REPORT at once; otherwise wait for the next pushed frame.
            found = d.get_input_report(0x03).and_then(|r| decode(&r)).or_else(|| wait_frame(&d));
            if found.is_some() {
                break;
            }
        }
        let s = match found {
            Some((level, charging)) => DeviceStatus {
                percent: level,
                charging,
                full: charging && level >= 100,
                ..base()
            },
            None => DeviceStatus { connected: false, ..base() },
        };
        ctx.add(s, vid, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_battery() {
        assert_eq!(decode(&[0x03, 0xE5, 0x40, 0x01, 0x4B]), Some((75, false)));
    }

    #[test]
    fn charging() {
        assert_eq!(decode(&[0x03, 0xE5, 0x40, 0x03, 0x64]), Some((100, true)));
    }

    #[test]
    fn link_blink_and_junk_skipped() {
        assert_eq!(decode(&[0x03, 0xE5, 0x01, 0x00, 0x00]), None);
        assert_eq!(decode(&[0x03, 0xE5, 0x40, 0x01, 0xC8]), None);
        assert_eq!(decode(&[0x03, 0xE5, 0x40]), None);
        assert_eq!(decode(&[0x02, 0xE5, 0x40, 0x01, 0x32]), None);
    }
}
