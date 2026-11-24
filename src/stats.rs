//! Statistics of one device from its battery history (see history.rs): discharge and charge
//! sessions, average rates, estimated battery life and time left.
//!
//! Everything comes from real readings. A session never spans a gap in monitoring (the START
//! flag), short or tiny sessions are left out (a 1 % step over a few minutes says nothing about
//! the rate), and a figure is None when there is not enough data for it; the dashboard then says
//! so instead of showing a made-up number.
use crate::history::Point;

/// A discharge session counts from this drop and this length on.
pub const MIN_DROP: u8 = 5;
pub const MIN_DISCHARGE_SECS: u64 = 30 * 60;
/// A charge session counts for the rate from this rise and this length on.
pub const MIN_RISE: u8 = 5;
pub const MIN_CHARGE_SECS: u64 = 5 * 60;
/// Averages need this much measured time in total.
pub const MIN_TOTAL_SECS: u64 = 3600;

/// One stretch on battery or on power, between two readings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Session {
    pub start: u64,
    pub end: u64,
    pub from: u8,
    pub to: u8,
    /// Charge sessions: reached 100 % or the device said "full".
    pub completed: bool,
}

impl Session {
    pub fn secs(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }
    /// Percent per hour (positive: charging, negative: discharging).
    pub fn rate(&self) -> f64 {
        let h = self.secs() as f64 / 3600.0;
        if h <= 0.0 { 0.0 } else { (self.to as f64 - self.from as f64) / h }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    /// Every session on battery (oldest first), counted or not.
    pub discharges: Vec<Session>,
    /// Every session on power (oldest first).
    pub charges: Vec<Session>,
    /// Average drop while on battery, % per hour.
    pub discharge_rate: Option<f64>,
    /// Average rise while charging, % per hour.
    pub charge_rate: Option<f64>,
    /// 100 % to 0 % at the average rate, in hours.
    pub battery_life_h: Option<f64>,
    /// From the current level to 0 % at the average rate (only while on battery), in hours.
    pub remaining_h: Option<f64>,
    /// 0 % to 100 % at the average charge rate, in hours.
    pub full_charge_h: Option<f64>,
    /// From the current level to 100 % (only while charging), in hours.
    pub to_full_h: Option<f64>,
    /// Average length of the charge sessions that counted, in hours.
    pub avg_charge_session_h: Option<f64>,
    /// End of the last charge session.
    pub last_charged: Option<u64>,
    pub readings: usize,
    /// Time covered by readings (gaps left out), in seconds.
    pub monitored_secs: u64,
    pub first_reading: Option<u64>,
}

/// Splits a run of points of one state into a session (first point to last point).
fn session(run: &[Point], charge: bool) -> Option<Session> {
    let (first, last) = (run.first()?, run.last()?);
    let to = if charge { run.iter().map(|p| p.level).max()? } else { last.level };
    // A charge session ends when it reached its highest level (the time on the charger after
    // that is "full", not charging).
    let end = if charge { run.iter().find(|p| p.level == to).map(|p| p.t)? } else { last.t };
    let completed = charge && (to >= 100 || run.iter().any(|p| p.full()));
    Some(Session { start: first.t, end, from: first.level, to, completed })
}

/// Statistics of `pts` (oldest first, as History::series returns them). `current` is the
/// device's level and charging state right now, if it is connected.
pub fn compute(pts: &[Point], current: Option<(i32, bool)>) -> Stats {
    let mut st = Stats { readings: pts.len(), first_reading: pts.first().map(|p| p.t), ..Default::default() };
    // Runs: consecutive points of the same state within one stretch of monitoring.
    let mut i = 0;
    while i < pts.len() {
        let charging = pts[i].charging();
        let mut j = i + 1;
        while j < pts.len() && !pts[j].starts() && pts[j].charging() == charging {
            j += 1;
        }
        // The next reading in the same stretch shows when this state ended: a discharge run ends
        // at the first reading on power (the level there is still the battery's), so it counts.
        let mut run: Vec<Point> = pts[i..j].to_vec();
        if !charging && j < pts.len() && !pts[j].starts() {
            run.push(Point { flags: 0, ..pts[j] });
        }
        if let Some(s) = session(&run, charging)
            && run.len() > 1
        {
            if charging { st.charges.push(s) } else { st.discharges.push(s) }
        }
        i = j;
    }
    // Monitored time: sum of the stretches.
    let mut seg_start = None;
    let mut prev: Option<&Point> = None;
    for p in pts {
        if p.starts() || seg_start.is_none() {
            if let (Some(s), Some(e)) = (seg_start, prev) {
                st.monitored_secs += e.t.saturating_sub(s);
            }
            seg_start = Some(p.t);
        }
        prev = Some(p);
    }
    if let (Some(s), Some(e)) = (seg_start, prev) {
        st.monitored_secs += e.t.saturating_sub(s);
    }

    let counted_dis: Vec<&Session> = st.discharges.iter().filter(|s| s.from >= s.to.saturating_add(MIN_DROP) && s.secs() >= MIN_DISCHARGE_SECS).collect();
    let (drop, secs) = counted_dis.iter().fold((0u64, 0u64), |(d, t), s| (d + (s.from - s.to) as u64, t + s.secs()));
    if secs >= MIN_TOTAL_SECS && drop >= MIN_DROP as u64 {
        let rate = drop as f64 / (secs as f64 / 3600.0);
        st.discharge_rate = Some(rate);
        st.battery_life_h = Some(100.0 / rate);
        if let Some((level, false)) = current
            && level >= 0
        {
            st.remaining_h = Some(level as f64 / rate);
        }
    }
    let counted_chg: Vec<&Session> = st.charges.iter().filter(|s| s.to >= s.from.saturating_add(MIN_RISE) && s.secs() >= MIN_CHARGE_SECS).collect();
    let (rise, secs) = counted_chg.iter().fold((0u64, 0u64), |(r, t), s| (r + (s.to - s.from) as u64, t + s.secs()));
    if !counted_chg.is_empty() && rise >= MIN_RISE as u64 && secs > 0 {
        let rate = rise as f64 / (secs as f64 / 3600.0);
        st.charge_rate = Some(rate);
        st.full_charge_h = Some(100.0 / rate);
        st.avg_charge_session_h = Some(secs as f64 / 3600.0 / counted_chg.len() as f64);
        if let Some((level, true)) = current
            && (0..100).contains(&level)
        {
            st.to_full_h = Some((100 - level) as f64 / rate);
        }
    }
    st.last_charged = st.charges.iter().filter(|s| s.to > s.from || s.completed).map(|s| s.end).max();
    st
}

/// "3 h 20 min", "45 min", "2 d 4 h" - rounded to what the data supports.
pub fn fmt_hours(h: f64) -> String {
    let mins = (h * 60.0).round() as i64;
    if mins < 1 {
        "under 1 min".into()
    } else if mins < 60 {
        format!("{mins} min")
    } else if mins < 48 * 60 {
        let (hh, mm) = (mins / 60, mins % 60);
        // Over 10 h, minutes are noise: round to the hour.
        if hh >= 10 || mm == 0 { format!("{} h", (mins + 30) / 60) } else { format!("{hh} h {mm} min") }
    } else {
        let hh = (mins + 30) / 60;
        if hh % 24 == 0 { format!("{} d", hh / 24) } else { format!("{} d {} h", hh / 24, hh % 24) }
    }
}

/// "12.5 %/h", "0.8 %/h".
pub fn fmt_rate(r: f64) -> String {
    if r >= 10.0 { format!("{r:.0} %/h") } else { format!("{r:.1} %/h") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{CHARGING, FULL, START};

    fn p(t_min: u64, level: u8, flags: u8) -> Point {
        Point { t: t_min * 60, level, flags }
    }

    #[test]
    fn not_enough_data_means_no_figures() {
        let st = compute(&[p(0, 80, START), p(20, 79, 0)], Some((79, false)));
        assert_eq!(st.discharge_rate, None);
        assert_eq!(st.remaining_h, None);
        assert_eq!(st.charge_rate, None);
        assert_eq!(compute(&[], None), Stats::default());
    }

    #[test]
    fn discharge_rate_life_and_time_left() {
        // 100 % -> 80 % in 4 hours: 5 %/h, 20 h of battery life, 16 h left at 80 %.
        let pts = [p(0, 100, START), p(60, 95, 0), p(120, 90, 0), p(180, 85, 0), p(240, 80, 0)];
        let st = compute(&pts, Some((80, false)));
        assert!((st.discharge_rate.unwrap() - 5.0).abs() < 1e-9);
        assert!((st.battery_life_h.unwrap() - 20.0).abs() < 1e-9);
        assert!((st.remaining_h.unwrap() - 16.0).abs() < 1e-9);
        assert_eq!(st.monitored_secs, 240 * 60);
        // While charging there is no "time left on battery".
        assert_eq!(compute(&pts, Some((80, true))).remaining_h, None);
    }

    #[test]
    fn gaps_split_sessions_and_are_not_counted() {
        // 50 -> 45 over 1 h, then the PC was off for 10 h and the level is 20 afterwards: the
        // drop during the gap is not measured, so it is not part of any session.
        let pts = [p(0, 50, START), p(60, 45, 0), p(660, 20, START), p(720, 15, 0)];
        let st = compute(&pts, None);
        assert_eq!(st.discharges.len(), 2);
        assert!((st.discharge_rate.unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(st.monitored_secs, 120 * 60);
    }

    #[test]
    fn charge_sessions_rates_and_completion() {
        let pts = [
            p(0, 60, START),
            p(60, 50, 0),
            p(61, 50, CHARGING),
            p(91, 80, CHARGING),
            p(121, 100, CHARGING | FULL),
            p(200, 100, CHARGING | FULL), // on the charger, already full
            p(201, 100, 0),
        ];
        let st = compute(&pts, Some((60, true)));
        assert_eq!(st.charges.len(), 1);
        let c = st.charges[0];
        assert!(c.completed);
        assert_eq!((c.from, c.to, c.secs()), (50, 100, 60 * 60)); // ends when it reached 100
        assert!((st.charge_rate.unwrap() - 50.0).abs() < 1e-9);
        assert!((st.full_charge_h.unwrap() - 2.0).abs() < 1e-9);
        assert!((st.to_full_h.unwrap() - 0.8).abs() < 1e-9);
        assert_eq!(st.last_charged, Some(121 * 60));
        // The discharge before it ends at the first reading on power.
        assert_eq!(st.discharges[0].end, 61 * 60);
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_hours(0.5), "30 min");
        assert_eq!(fmt_hours(3.34), "3 h 20 min");
        assert_eq!(fmt_hours(12.4), "12 h");
        assert_eq!(fmt_hours(52.0), "2 d 4 h");
        assert_eq!(fmt_rate(4.26), "4.3 %/h");
        assert_eq!(fmt_rate(12.4), "12 %/h");
    }
}
