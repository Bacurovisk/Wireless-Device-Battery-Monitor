//! Logitech G435 LIGHTSPEED headset on its newer 2.4 GHz dongle (VID 046D, PID 0ADF).
//!
//! Not HID++: the dongle's vendor collection FF00:0001 (unnumbered 64-byte reports) carries CRC-16
//! framed messages, and "PA" commands inside them reach the headset's radio chip. Protocol from
//! g435-battery-percentage (MIT, github.com/portyporty/g435-battery-percentage, a tray app made from
//! a capture on this headset), as also used by TaskbarVitals (GPL-3.0, github.com/symNistaken/
//! TaskbarVitals, src/drivers/logitech_g435_0adf.cpp, where it was checked on hardware that a read
//! does not interrupt audio or the microphone); implemented here from those facts.
//!
//!   frame    <cmd> <body length> <crc lo> <crc hi> <body ..>   64 bytes, CRC-16/XMODEM over the frame
//!                                                              with the CRC field zeroed
//!   PA       50 41 <category> <seq> <length> <payload ..>
//!   session  84 01, 85 01, session init, wireless GET_MODE / SET_MODE 01 / GET_MODE (routes the
//!            next command to the headset; not a setting, not kept), OTA GET_BATTERY, 85 02
//!   reply    .. 00 07 00 06 <seq> 00 <mV lo> <mV hi> ..
//!
//! The level comes from the voltage (g435-battery-percentage's Li-Po curve). The older 046D:0ACB
//! dongle uses another transport and is not touched. Read every 5 minutes, as the reference does;
//! no reply means the headset is off or out of range.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::Deadline;

const VID: u16 = 0x046D;
const PID: u16 = 0x0ADF;
const CAT_OTA: u8 = 0x06;
const CAT_WIRELESS: u8 = 0x0E;
const GET_MODE: u8 = 0xE0;
const SET_MODE: u8 = 0xE1;
const QUERY_EVERY: Duration = Duration::from_secs(5 * 60);
const OFF_QUERY_EVERY: Duration = Duration::from_secs(60);
/// Session init from the capture (OTA IOCTL_READ_MAX_LEN, sequence 1 inside).
const SESSION_INIT: [u8; 19] = [0x50, 0x41, 0x06, 0x01, 0x0D, 0x00, 0x25, 0x34, 0x12, 0x5A, 0x5A, 0x01, 0x00, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x00];

/// CRC-16/XMODEM (poly 1021, init 0).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// One 64-byte frame.
pub fn frame(cmd: u8, body: &[u8]) -> [u8; 64] {
    let mut f = [0u8; 64];
    f[0] = cmd;
    f[1] = body.len() as u8;
    let n = body.len().min(60);
    f[4..4 + n].copy_from_slice(&body[..n]);
    let crc = crc16(&f);
    f[2] = (crc & 0xFF) as u8;
    f[3] = (crc >> 8) as u8;
    f
}

pub fn pa(category: u8, seq: u8, len: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0x50, 0x41, category, seq, len];
    v.extend_from_slice(payload);
    v
}

/// Battery voltage from a GET_BATTERY reply; None when absent or not a plausible Li-Po voltage.
pub fn parse_mv(r: &[u8]) -> Option<i32> {
    r.windows(8).find_map(|w| {
        (w[0] == 0x00 && w[1] == 0x07 && w[2] == 0x00 && w[3] == 0x06 && w[5] == 0x00)
            .then(|| w[6] as i32 | (w[7] as i32) << 8)
            .filter(|mv| (2500..=4500).contains(mv))
    })
}

/// Voltage -> % (g435-battery-percentage's curve), piecewise linear.
pub fn percent(mv: i32) -> i32 {
    const V: [i32; 16] = [4200, 4150, 4100, 4050, 4000, 3950, 3900, 3850, 3800, 3750, 3700, 3650, 3600, 3550, 3500, 3400];
    const P: [i32; 16] = [100, 95, 90, 85, 80, 75, 70, 62, 55, 45, 35, 25, 15, 10, 5, 0];
    if mv >= V[0] {
        return 100;
    }
    for i in 1..16 {
        if mv >= V[i] {
            return P[i] + ((P[i - 1] - P[i]) * (mv - V[i]) + (V[i - 1] - V[i]) / 2) / (V[i - 1] - V[i]);
        }
    }
    0
}

/// Sends one frame (report id 0 first) and reads for `wait_ms`; with `want_mv`, stops at a battery reply.
fn transact(d: &HidDev, cmd: u8, body: &[u8], wait_ms: u64, want_mv: bool) -> Option<Option<i32>> {
    let f = frame(cmd, body);
    let mut out = Vec::with_capacity(65);
    out.push(0x00);
    out.extend_from_slice(&f);
    d.flush();
    if !d.write(&out) {
        return None;
    }
    let end = Deadline::in_ms(wait_ms);
    while !end.passed() {
        if let Some(r) = d.read(end.left_ms())
            && want_mv
            && let Some(mv) = parse_mv(&r)
        {
            return Some(Some(mv));
        }
    }
    Some(None)
}

fn read_mv(c: &HidColl) -> Option<Option<i32>> {
    let d = HidDev::open(c)?;
    transact(&d, 0x84, &[0x01], 50, false)?;
    let mut seq: u8 = 2;
    let mut next = |cat: u8, len: u8, payload: &[u8]| {
        let cmd = pa(cat, seq, len, payload);
        seq = seq % 127 + 1;
        cmd
    };
    let steps = (|| {
        transact(&d, 0x85, &[0x01], 50, false)?;
        transact(&d, 0x80, &SESSION_INIT, 450, false)?;
        transact(&d, 0x80, &next(CAT_WIRELESS, 1, &[GET_MODE]), 120, false)?;
        transact(&d, 0x80, &next(CAT_WIRELESS, 2, &[SET_MODE, 0x01]), 120, false)?;
        transact(&d, 0x80, &next(CAT_WIRELESS, 1, &[GET_MODE]), 120, false)?;
        transact(&d, 0x80, &next(CAT_OTA, 1, &[0x00, 0x31]), 250, true)
    })();
    // The session is always closed once it was opened.
    let _ = transact(&d, 0x85, &[0x02], 50, false);
    Some(steps.flatten())
}

#[derive(Clone, Copy)]
struct Last {
    at: Instant,
    mv: Option<i32>,
}

static LAST: Mutex<Option<Last>> = Mutex::new(None);

pub fn poll(ctx: &mut PollContext) {
    if ctx.is_claimed(VID, PID) {
        return;
    }
    let colls = ctx.find(VID, PID, |c| c.page() == 0xFF00 && c.usage() == 0x0001 && c.in_len() == 65 && c.out_len() == 65);
    if colls.is_empty() {
        *LAST.lock().unwrap_or_else(|e| e.into_inner()) = None;
        return;
    }
    let cached = *LAST.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = cached.filter(|l| l.at.elapsed() < if l.mv.is_some() { QUERY_EVERY } else { OFF_QUERY_EVERY });
    let mv = match fresh {
        Some(l) => l.mv,
        None => {
            let Some(mv) = colls.iter().find_map(read_mv) else { return };
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(Last { at: Instant::now(), mv });
            mv
        }
    };
    let base = DeviceStatus {
        key: "g435:0adf".into(),
        name: "Logitech G435".into(),
        kind: DeviceKind::Headset,
        via: "2.4 GHz receiver".into(),
        hw: Some((VID, PID)),
        grace_sec: 60,
        ..Default::default()
    };
    let s = match mv {
        Some(mv) => DeviceStatus { percent: percent(mv), millivolts: mv, ..base },
        None => DeviceStatus { connected: false, ..base },
    };
    ctx.add(s, VID, PID);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames() {
        assert_eq!(crc16(b"123456789"), 0x31C3); // CRC-16/XMODEM check value
        let f = frame(0x84, &[0x01]);
        assert_eq!(&f[..2], &[0x84, 0x01]);
        assert_eq!(f[4], 0x01);
        let mut z = f;
        z[2] = 0;
        z[3] = 0;
        assert_eq!(crc16(&z), u16::from_le_bytes([f[2], f[3]]));
        assert_eq!(pa(CAT_OTA, 7, 1, &[0x00, 0x31]), vec![0x50, 0x41, 0x06, 7, 1, 0x00, 0x31]);
    }

    #[test]
    fn replies_and_curve() {
        let mut r = vec![0u8; 64];
        r[10..18].copy_from_slice(&[0x00, 0x07, 0x00, 0x06, 0x09, 0x00, 0x68, 0x0F]); // 3944 mV
        assert_eq!(parse_mv(&r), Some(3944));
        r[16] = 0x10;
        r[17] = 0x00; // 16 mV: not a voltage
        assert_eq!(parse_mv(&r), None);
        assert_eq!(percent(4250), 100);
        assert_eq!(percent(3300), 0);
        assert_eq!(percent(3800), 55);
        assert!(percent(3944) > 70 && percent(3944) < 75);
    }
}
