//! Razer BlackShark V3 headset over its 2.4 GHz HyperSpeed dongle (VID 1532, PID 057A).
//!
//! Protocol as documented by OpenRazer PR #2794 and Joule PR #36 (reverse engineered from
//! captures). The dongle's vendor collection (usage page FF14, 64-byte input/output reports,
//! report ID 02) carries commands with an XOR checksum of bytes 0..61 at byte 62:
//!
//!   battery GET   02 00 60 00 00 00 04 00 00 80 21 00 ... [62]=C7 [63]=00
//!   reply / push  02 .. .. .. .. .. .. .. .. .. 21 0x 01 NN      NN = level 0..100, FF = not known yet
//!   charging push 02 .. .. .. .. .. .. .. .. .. 2A .. .. CC      CC = 0 / 1
//!
//! The headset also pushes the level by itself now and then (on change, on power on / off). Asking
//! too often is reported to make the headset restart, so BatteryHub asks at most every
//! QUERY_EVERY and otherwise listens: one background thread per dongle keeps the collection open
//! and records every report; the regular poll only reads what it recorded. A query that gets no
//! reply means the headset is off or out of range.
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::device::{DeviceKind, DeviceStatus, PollContext};
use crate::hid::{HidColl, HidDev};
use crate::util::hex4;

const VID: u16 = 0x1532;
const BLACKSHARK_V3: u16 = 0x057A;
const QUERY_EVERY: Duration = Duration::from_secs(30 * 60);
/// A reply comes within milliseconds while the headset is linked.
const REPLY_WAIT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct State {
    running: bool,
    level: Option<i32>,
    charging: bool,
    last_query: Option<Instant>,
    last_reply: Option<Instant>,
    /// A query was sent and nothing came back: headset off.
    silent: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn battery_request() -> [u8; 64] {
    let mut r = [0u8; 64];
    r[0] = 0x02;
    r[2] = 0x60;
    r[6] = 0x04;
    r[9] = 0x80;
    r[10] = 0x21;
    r[62] = r[..62].iter().fold(0, |a, b| a ^ b);
    r
}

#[derive(Debug, PartialEq)]
pub enum Report {
    Level(i32),
    Unknown,
    Charging(bool),
}

/// Decodes one input report (report ID at index 0).
pub fn decode(r: &[u8]) -> Option<Report> {
    if r.len() < 14 || r[0] != 0x02 {
        return None;
    }
    match r[10] {
        0x21 if matches!(r[11], 0x01 | 0x02) && r[12] == 0x01 => match r[13] {
            0xFF => Some(Report::Unknown),
            n if n <= 100 => Some(Report::Level(n as i32)),
            _ => None,
        },
        0x2A if r[13] <= 1 => Some(Report::Charging(r[13] == 1)),
        _ => None,
    }
}

fn listener(c: HidColl) {
    let Some(d) = HidDev::open(&c) else {
        with(|s| s.running = false);
        return;
    };
    let mut failures = 0;
    loop {
        let due = with(|s| s.last_query.map_or(true, |t| t.elapsed() >= QUERY_EVERY));
        if due {
            let sent = d.write(&battery_request());
            with(|s| s.last_query = Some(Instant::now()));
            if !sent {
                failures += 1;
            }
        }
        let started = Instant::now();
        match d.read(5_000) {
            Some(r) => {
                failures = 0;
                match decode(&r) {
                    Some(Report::Level(n)) => with(|s| {
                        s.level = Some(n);
                        s.last_reply = Some(Instant::now());
                        s.silent = false;
                    }),
                    Some(Report::Unknown) => with(|s| {
                        s.last_reply = Some(Instant::now());
                        s.silent = false;
                    }),
                    Some(Report::Charging(on)) => with(|s| s.charging = on),
                    None => {}
                }
            }
            // A timeout takes the full 5 s; an error returns at once (dongle unplugged).
            None if started.elapsed() < Duration::from_secs(1) => failures += 1,
            None => {}
        }
        with(|s| {
            if let Some(q) = s.last_query {
                if q.elapsed() > REPLY_WAIT && s.last_reply.map_or(true, |r| r < q) {
                    s.silent = true;
                }
            }
        });
        if failures >= 5 {
            with(|s| *s = State::default());
            return;
        }
        if failures > 0 {
            crate::util::sleep_ms(1000);
        }
    }
}

pub fn poll(ctx: &mut PollContext) {
    for (vid, pid) in ctx.products(VID) {
        if pid != BLACKSHARK_V3 {
            continue;
        }
        let Some(c) = ctx.find(vid, pid, |c| c.page() == 0xFF14 && c.in_len() == 64 && c.out_len() == 64).into_iter().next() else {
            continue;
        };
        let started = with(|s| !std::mem::replace(&mut s.running, true));
        if started {
            std::thread::spawn(move || listener(c));
            // The first answer comes within milliseconds while the headset is linked.
            let end = Instant::now() + REPLY_WAIT;
            while Instant::now() < end && with(|s| s.level.is_none() && !s.silent) {
                crate::util::sleep_ms(100);
            }
        }
        let (level, charging, silent) = with(|s| (s.level, s.charging, s.silent));
        let mut st = DeviceStatus {
            key: format!("razer:{}", hex4(pid)),
            name: "Razer BlackShark V3".into(),
            kind: DeviceKind::Headset,
            via: "2.4 GHz dongle".into(),
            hw: Some((vid, pid)),
            grace_sec: 60,
            ..Default::default()
        };
        match level {
            Some(n) if !silent => {
                st.percent = n;
                st.charging = charging;
            }
            _ => st.connected = false,
        }
        ctx.add(st, vid, pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_checksum() {
        let r = battery_request();
        assert_eq!(&r[..11], &[0x02, 0x00, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x80, 0x21]);
        assert_eq!(r[62], 0xC7);
        assert_eq!(r[63], 0x00);
    }

    fn rep(class: u8, b11: u8, b12: u8, b13: u8) -> Vec<u8> {
        let mut r = vec![0u8; 64];
        r[0] = 0x02;
        r[10] = class;
        r[11] = b11;
        r[12] = b12;
        r[13] = b13;
        r
    }

    #[test]
    fn reports() {
        assert_eq!(decode(&rep(0x21, 0x01, 0x01, 35)), Some(Report::Level(35)));
        assert_eq!(decode(&rep(0x21, 0x02, 0x01, 69)), Some(Report::Level(69)));
        assert_eq!(decode(&rep(0x21, 0x01, 0x01, 0xFF)), Some(Report::Unknown));
        assert_eq!(decode(&rep(0x2A, 0x00, 0x00, 1)), Some(Report::Charging(true)));
        assert_eq!(decode(&rep(0x21, 0x01, 0x01, 150)), None);
        assert_eq!(decode(&rep(0x20, 0x01, 0x01, 50)), None);
    }
}
