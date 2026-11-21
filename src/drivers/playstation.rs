//! Sony DualShock 4 / DualSense / DualSense Edge (VID 054C), USB or Bluetooth.
//! The battery is part of the regular input report:
//!   DualShock 4: USB report 01 byte 30, Bluetooth report 11 byte 32: bits 0-3 level, bit 4 cable
//!   DualSense:   USB report 01 byte 53, Bluetooth report 31 byte 54: bits 0-3 level (0-10), bits 4-7 state
//! Over Bluetooth both send a reduced report until a feature report (05, calibration) is read.
//! Layout reference: Linux drivers/hid/hid-playstation.c.
use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::HidDev;
use crate::util::{clamp100, fnv1a, hex4, Deadline};

fn is_ds4(pid: u16) -> bool {
    matches!(pid, 0x05C4 | 0x09CC | 0x0BA0)
}
fn is_dualsense(pid: u16) -> bool {
    matches!(pid, 0x0CE6 | 0x0DF2)
}

pub fn poll(ctx: &mut PollContext) {
    let colls: Vec<_> = ctx
        .colls
        .iter()
        .filter(|c| c.vid == 0x054C && (is_ds4(c.pid) || is_dualsense(c.pid)) && c.page() == 0x01 && c.usage() == 0x05)
        .cloned()
        .collect();
    for c in colls {
        let Some(d) = HidDev::open(&c) else { continue };
        let bt = c.in_len() > 64;
        if bt {
            let _ = d.get_feature(0x05); // switches the controller to full reports
        }
        let (mut pct, mut charging, mut found) = (-1, false, false);
        let end = Deadline::in_ms(500);
        while !found && !end.passed() {
            let Some(r) = d.read(100).filter(|r| r.len() >= 10) else { continue };
            if is_ds4(c.pid) {
                let at = match r[0] {
                    0x01 => 30,
                    0x11 => 32,
                    _ => continue,
                };
                if r.len() <= at || (r[0] == 0x01 && bt) {
                    continue;
                }
                let level = (r[at] & 0x0F) as i32;
                let cable = r[at] & 0x10 != 0;
                if c.pid == 0x0BA0 && r[at] == 0 {
                    break; // wireless adapter without a controller
                }
                pct = if cable { if level >= 11 { 100 } else { clamp100(level * 10) } } else { clamp100(level * 10 + 5) };
                charging = cable && level < 11;
                found = true;
            } else {
                let at = match r[0] {
                    0x01 => 53,
                    0x31 => 54,
                    _ => continue,
                };
                if r.len() <= at || (r[0] == 0x01 && bt) {
                    continue;
                }
                let level = (r[at] & 0x0F) as i32;
                let state = r[at] >> 4;
                pct = clamp100(level * 10 + 5);
                charging = state == 0x1;
                if state == 0x2 {
                    pct = 100;
                    charging = true;
                }
                found = true;
            }
        }
        if !found {
            continue;
        }
        let s = DeviceStatus {
            key: format!("ps:{}:{}", hex4(c.pid), fnv1a(&c.path)),
            name: if is_ds4(c.pid) {
                "DualShock 4"
            } else if c.pid == 0x0DF2 {
                "DualSense Edge"
            } else {
                "DualSense"
            }
            .into(),
            kind: DeviceKind::Gamepad,
            via: if bt {
                "Bluetooth"
            } else if c.pid == 0x0BA0 {
                "Wireless adapter"
            } else {
                "USB cable"
            }
            .into(),
            percent: pct,
            charging,
            full: charging && pct >= 100,
            vid: 0x054C,
            ..Default::default()
        };
        ctx.out.push(s);
        ctx.claim(c.vid, c.pid);
    }
}
