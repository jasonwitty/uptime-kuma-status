//! The flattened view of a status page that the UI draws.
//!
//! Groups become a flat monitor list plus a list of group names; everything
//! the renderer needs is precomputed here so `ui.rs` only lays out spans.

use std::time::Instant;

use ratatui::style::Color;

use crate::api::{Beat, ConfigResponse, HeartbeatResponse};

/// A beat within this many seconds of the newest beat counts as "recent".
const RECENT_WINDOW_SECS: i64 = 600;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Status {
    Up,
    Down,
    Pending,
    Maintenance,
    Unknown,
}

impl Status {
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => Status::Down,
            1 => Status::Up,
            2 => Status::Pending,
            3 => Status::Maintenance,
            _ => Status::Unknown,
        }
    }

    /// Named colors only, so the user's terminal theme is the theme.
    pub fn color(self) -> Color {
        match self {
            Status::Up => Color::Green,
            Status::Down => Color::Red,
            Status::Pending => Color::Yellow,
            Status::Maintenance => Color::Blue,
            Status::Unknown => Color::DarkGray,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Monitor {
    pub name: String,
    /// Index into [`State::groups`].
    pub group: usize,
    /// Oldest first, at most ~50.
    pub beats: Vec<Beat>,
    /// 24 hour uptime as a ratio in `0.0..=1.0`.
    pub uptime24: Option<f64>,
    pub current: Status,
    pub latest_ping: Option<f64>,
    /// Went down at least once within [`RECENT_WINDOW_SECS`] of its newest beat.
    pub recently_down: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Overall {
    Operational,
    PartiallyDegraded,
    Degraded,
    Maintenance,
    Pending,
    Unknown,
}

/// A fetch result handed from the worker thread to the UI thread.
pub enum Msg {
    Data(Box<ConfigResponse>, Box<HeartbeatResponse>),
    Error(String),
}

#[derive(Debug)]
pub struct State {
    pub title: String,
    pub groups: Vec<String>,
    /// Page order, exactly as the status page lists them.
    pub monitors: Vec<Monitor>,
    pub incident: Option<(String, Color)>,
    pub maintenance: Vec<String>,
    pub last_ok: Option<Instant>,
    pub error: Option<String>,
}

impl State {
    pub fn new(title: String) -> Self {
        State {
            title,
            groups: Vec::new(),
            monitors: Vec::new(),
            incident: None,
            maintenance: Vec::new(),
            last_ok: None,
            error: None,
        }
    }

    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Data(cfg, hb) => {
                if !cfg.config.title.is_empty() {
                    self.title = cfg.config.title.clone();
                }
                self.groups = cfg
                    .public_group_list
                    .iter()
                    .map(|g| g.name.clone())
                    .collect();
                self.monitors = build_monitors(&cfg, &hb);
                self.incident = cfg
                    .incident
                    .as_ref()
                    .map(|i| (i.title.clone(), incident_color(i.style.as_deref())));
                self.maintenance = cfg
                    .maintenance_list
                    .iter()
                    .map(|m| m.title.clone())
                    .collect();
                self.last_ok = Some(Instant::now());
                self.error = None;
            }
            Msg::Error(e) => self.error = Some(e),
        }
    }

    pub fn overall(&self) -> Overall {
        let n = self.monitors.len();
        if n == 0 {
            return Overall::Unknown;
        }
        let down = self
            .monitors
            .iter()
            .filter(|m| m.current == Status::Down)
            .count();
        if down == n {
            Overall::Degraded
        } else if down > 0 {
            // Red whenever anything is down: a wall display has to be glanceable.
            Overall::PartiallyDegraded
        } else if self
            .monitors
            .iter()
            .any(|m| m.current == Status::Maintenance)
        {
            Overall::Maintenance
        } else if self.monitors.iter().any(|m| m.current == Status::Pending) {
            Overall::Pending
        } else {
            Overall::Operational
        }
    }

    /// Indices into [`State::monitors`], most worth seeing first.
    ///
    /// When the screen cannot hold everything, whatever is broken - or was
    /// broken in the last ten minutes - must survive the truncation. If
    /// everything is broken, or nothing is, that ordering carries no
    /// information, so page order is kept instead.
    pub fn display_order(&self) -> Vec<usize> {
        let hot = |m: &Monitor| m.current != Status::Up || m.recently_down;
        let n_hot = self.monitors.iter().filter(|m| hot(m)).count();
        if n_hot == 0 || n_hot == self.monitors.len() {
            return (0..self.monitors.len()).collect();
        }
        let (mut first, rest): (Vec<usize>, Vec<usize>) =
            (0..self.monitors.len()).partition(|&i| hot(&self.monitors[i]));
        first.extend(rest);
        first
    }
}

fn incident_color(style: Option<&str>) -> Color {
    match style {
        Some("danger") => Color::Red,
        Some("warning") => Color::Yellow,
        _ => Color::Blue,
    }
}

pub fn build_monitors(cfg: &ConfigResponse, hb: &HeartbeatResponse) -> Vec<Monitor> {
    let mut out = Vec::new();
    for (group, g) in cfg.public_group_list.iter().enumerate() {
        for m in &g.monitor_list {
            let beats = hb
                .heartbeat_list
                .get(&m.id.to_string())
                .cloned()
                .unwrap_or_default();
            let last = beats.last();
            out.push(Monitor {
                name: m.name.clone(),
                group,
                current: last.map_or(Status::Unknown, |b| Status::from_code(b.status)),
                latest_ping: last.and_then(|b| b.ping),
                recently_down: recently_down(&beats),
                uptime24: hb.uptime_list.get(&format!("{}_24", m.id)).copied(),
                beats,
            });
        }
    }
    out
}

/// Measured against the newest beat's own clock rather than the system clock,
/// so the UTC timestamps never need a timezone conversion.
fn recently_down(beats: &[Beat]) -> bool {
    let Some(newest) = beats.last().and_then(|b| parse_ts(&b.time)) else {
        return false;
    };
    beats.iter().any(|b| {
        b.status == 0 && parse_ts(&b.time).is_some_and(|t| newest - t <= RECENT_WINDOW_SECS)
    })
}

/// Low and high ping bounds for a monitor's heartbeat bar.
///
/// Scaling naively from zero to the maximum reads badly on real data: a single
/// latency spike squashes every other beat into one flat band, and a monitor
/// that is simply slow but steady pins at full height forever. So the top of
/// the scale is the 95th percentile - the spike still saturates, but it no
/// longer sets the scale for everything else.
///
/// The range is then floored, because the opposite failure is just as bad: a
/// monitor pinging 0-1 ms is genuinely stable, and stretching that jitter
/// across all eight levels would invent drama that is not there.
pub fn ping_scale(beats: &[Beat]) -> (f64, f64) {
    let mut pings: Vec<f64> = beats.iter().filter_map(|b| b.ping).collect();
    if pings.is_empty() {
        return (0.0, 1.0);
    }
    pings.sort_by(f64::total_cmp);

    let lo = pings[0];
    let idx = ((pings.len() * 95) / 100)
        .saturating_sub(1)
        .min(pings.len() - 1);
    let p95 = pings[idx];
    let hi = p95.max(lo + (lo * 0.25).max(2.0));
    (lo, hi)
}

/// Which of the eight bar heights a ping belongs in. Missing ping sits at the
/// bottom rather than inventing a height.
pub fn ping_level(ping: Option<f64>, lo: f64, hi: f64) -> usize {
    let Some(p) = ping else { return 0 };
    if hi <= lo {
        return 0;
    }
    (((p - lo) / (hi - lo)) * 7.0).round().clamp(0.0, 7.0) as usize
}

/// `"YYYY-MM-DD HH:MM:SS[.mmm]"` to seconds since the Unix epoch.
///
/// Uses Howard Hinnant's days-from-civil algorithm, which is short enough that
/// a date/time crate would cost more than it saves.
pub fn parse_ts(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    // The date/time separator is a space in the API's own format, but accept
    // the ISO-8601 'T' too rather than break on a future version.
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || (b[10] != b' ' && b[10] != b'T')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let field = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let (hh, mm, ss) = (field(11..13)?, field(14..16)?, field(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    if hh > 23 || mm > 59 || ss > 60 {
        return None;
    }

    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    Some(days * 86_400 + hh * 3_600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beat(status: u8, time: &str) -> Beat {
        Beat {
            status,
            time: time.to_string(),
            ping: Some(10.0),
        }
    }

    fn monitor(name: &str, current: Status, recently_down: bool) -> Monitor {
        Monitor {
            name: name.into(),
            group: 0,
            beats: Vec::new(),
            uptime24: None,
            current,
            latest_ping: None,
            recently_down,
        }
    }

    fn state(monitors: Vec<Monitor>) -> State {
        let mut s = State::new("t".into());
        s.monitors = monitors;
        s
    }

    fn beats_with_pings(pings: &[f64]) -> Vec<Beat> {
        pings
            .iter()
            .map(|p| Beat {
                status: 1,
                time: "2026-08-26 20:00:00".into(),
                ping: Some(*p),
            })
            .collect()
    }

    #[test]
    fn one_spike_does_not_flatten_the_rest_of_the_bar() {
        // 49 beats clustered near 200 ms, one 1128 ms outlier.
        let mut pings = vec![191.0; 10];
        pings.extend([210.0; 10]);
        pings.extend([250.0; 10]);
        pings.extend([288.0; 19]);
        pings.push(1128.0);
        let beats = beats_with_pings(&pings);
        let (lo, hi) = ping_scale(&beats);

        assert_eq!(lo, 191.0);
        assert!(hi < 400.0, "p95 should cap the scale, got {hi}");
        assert_eq!(
            ping_level(Some(1128.0), lo, hi),
            7,
            "the spike still saturates"
        );
        assert_eq!(ping_level(Some(191.0), lo, hi), 0);
        // The cluster spreads across levels instead of collapsing into one.
        let levels: Vec<usize> = [210.0, 250.0, 288.0]
            .iter()
            .map(|p| ping_level(Some(*p), lo, hi))
            .collect();
        assert!(levels[0] < levels[2], "cluster should spread: {levels:?}");
    }

    #[test]
    fn a_genuinely_stable_monitor_stays_calm() {
        // 0-1 ms jitter must not be amplified into peaks.
        let beats = beats_with_pings(&[0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]);
        let (lo, hi) = ping_scale(&beats);
        let levels: Vec<usize> = beats.iter().map(|b| ping_level(b.ping, lo, hi)).collect();
        assert!(
            levels.iter().all(|&l| l <= 4),
            "flat monitor amplified into {levels:?}"
        );
    }

    #[test]
    fn ping_scale_survives_empty_and_missing_pings() {
        assert_eq!(ping_scale(&[]), (0.0, 1.0));
        let beats = vec![Beat {
            status: 1,
            time: "x".into(),
            ping: None,
        }];
        let (lo, hi) = ping_scale(&beats);
        assert_eq!(ping_level(None, lo, hi), 0);
    }

    #[test]
    fn ping_level_is_always_in_range() {
        let beats = beats_with_pings(&[10.0, 20.0, 30.0]);
        let (lo, hi) = ping_scale(&beats);
        for p in [-5.0, 0.0, 10.0, 25.0, 1e9] {
            assert!(ping_level(Some(p), lo, hi) <= 7);
        }
    }

    #[test]
    fn parse_ts_matches_known_epochs() {
        assert_eq!(parse_ts("1970-01-01 00:00:00"), Some(0));
        assert_eq!(parse_ts("2000-01-01 00:00:00"), Some(946_684_800));
        // 2026-08-26 20:08:39 UTC
        assert_eq!(parse_ts("2026-08-26 20:08:39.799"), Some(1_787_774_919));
    }

    #[test]
    fn parse_ts_rejects_junk() {
        for s in [
            "",
            "nope",
            "2026-13-01 00:00:00",
            "2026/08/26 20:08:39",
            "2026-08-26 99:00:00",
        ] {
            assert_eq!(parse_ts(s), None, "should have rejected {s}");
        }
    }

    #[test]
    fn parse_ts_accepts_the_iso_separator_too() {
        assert_eq!(
            parse_ts("2026-08-26T20:08:39"),
            parse_ts("2026-08-26 20:08:39")
        );
    }

    #[test]
    fn recently_down_uses_the_beat_clock() {
        // Down 5 minutes before the newest beat: recent.
        assert!(recently_down(&[
            beat(0, "2026-08-26 20:00:00"),
            beat(1, "2026-08-26 20:05:00"),
        ]));
        // Down 30 minutes before the newest beat: not recent.
        assert!(!recently_down(&[
            beat(0, "2026-08-26 19:35:00"),
            beat(1, "2026-08-26 20:05:00"),
        ]));
        assert!(!recently_down(&[]));
    }

    #[test]
    fn builds_monitors_from_the_real_fixtures() {
        let cfg: ConfigResponse =
            serde_json::from_str(include_str!("../tests/fixtures/config.json")).unwrap();
        let hb: HeartbeatResponse =
            serde_json::from_str(include_str!("../tests/fixtures/heartbeat.json")).unwrap();
        let monitors = build_monitors(&cfg, &hb);

        assert_eq!(monitors.len(), 6, "3 monitors in each of 2 groups");
        assert_eq!(monitors[0].name, "socktop.io");
        assert_eq!(monitors[0].group, 0);
        assert_eq!(monitors[5].group, 1, "second group is indexed 1");
        assert_eq!(monitors[0].current, Status::Up);
        assert_eq!(monitors[0].beats.len(), 5);
        assert!(monitors[0].uptime24.is_some());
        assert!(monitors[0].latest_ping.is_some());
        assert!(!monitors[0].recently_down);
    }

    #[test]
    fn a_monitor_with_no_heartbeats_is_unknown_not_a_panic() {
        let cfg: ConfigResponse =
            serde_json::from_str(include_str!("../tests/fixtures/config.json")).unwrap();
        let hb: HeartbeatResponse = serde_json::from_str("{}").unwrap();
        let monitors = build_monitors(&cfg, &hb);
        assert_eq!(monitors.len(), 6);
        assert_eq!(monitors[0].current, Status::Unknown);
        assert_eq!(monitors[0].uptime24, None);
        assert!(monitors[0].beats.is_empty());
    }

    #[test]
    fn incident_style_picks_a_named_colour() {
        assert_eq!(incident_color(Some("danger")), Color::Red);
        assert_eq!(incident_color(Some("warning")), Color::Yellow);
        assert_eq!(incident_color(Some("info")), Color::Blue);
        assert_eq!(incident_color(None), Color::Blue);
        assert_eq!(incident_color(Some("something-new")), Color::Blue);
    }

    #[test]
    fn banners_and_title_come_through_from_the_api() {
        let cfg: ConfigResponse = serde_json::from_str(
            r#"{"config":{"title":"Mock"},
                "incident":{"title":"Disk full","style":"danger"},
                "maintenanceList":[{"title":"Rack B"},{"title":"Rack C"}],
                "publicGroupList":[]}"#,
        )
        .unwrap();
        let hb: HeartbeatResponse = serde_json::from_str("{}").unwrap();

        let mut state = State::new("placeholder".into());
        state.apply(Msg::Data(Box::new(cfg), Box::new(hb)));

        assert_eq!(state.title, "Mock");
        assert_eq!(state.incident, Some(("Disk full".to_string(), Color::Red)));
        assert_eq!(state.maintenance, vec!["Rack B", "Rack C"]);
        assert!(state.error.is_none());
        assert!(state.last_ok.is_some());
    }

    #[test]
    fn a_failed_fetch_keeps_the_last_good_data() {
        let cfg: ConfigResponse =
            serde_json::from_str(include_str!("../tests/fixtures/config.json")).unwrap();
        let hb: HeartbeatResponse =
            serde_json::from_str(include_str!("../tests/fixtures/heartbeat.json")).unwrap();
        let mut state = State::new("t".into());
        state.apply(Msg::Data(Box::new(cfg), Box::new(hb)));
        let before = state.monitors.len();

        state.apply(Msg::Error("cannot reach host".into()));
        assert_eq!(
            state.monitors.len(),
            before,
            "stale data is better than none"
        );
        assert_eq!(state.error.as_deref(), Some("cannot reach host"));
        assert!(
            state.last_ok.is_some(),
            "the age of the good data still counts"
        );
    }

    #[test]
    fn overall_covers_every_state() {
        use Status::*;
        assert_eq!(state(vec![]).overall(), Overall::Unknown);
        assert_eq!(
            state(vec![monitor("a", Up, false)]).overall(),
            Overall::Operational
        );
        assert_eq!(
            state(vec![monitor("a", Down, false)]).overall(),
            Overall::Degraded
        );
        assert_eq!(
            state(vec![monitor("a", Down, false), monitor("b", Up, false)]).overall(),
            Overall::PartiallyDegraded
        );
        assert_eq!(
            state(vec![
                monitor("a", Maintenance, false),
                monitor("b", Up, false)
            ])
            .overall(),
            Overall::Maintenance
        );
        assert_eq!(
            state(vec![monitor("a", Pending, false), monitor("b", Up, false)]).overall(),
            Overall::Pending
        );
    }

    #[test]
    fn display_order_promotes_trouble_but_only_when_it_distinguishes() {
        use Status::*;
        // Nothing wrong: page order.
        let s = state(vec![monitor("a", Up, false), monitor("b", Up, false)]);
        assert_eq!(s.display_order(), vec![0, 1]);

        // Everything wrong: page order again.
        let s = state(vec![monitor("a", Down, false), monitor("b", Down, false)]);
        assert_eq!(s.display_order(), vec![0, 1]);

        // One down in the middle: it goes first, the rest keep their order.
        let s = state(vec![
            monitor("a", Up, false),
            monitor("b", Down, false),
            monitor("c", Up, false),
        ]);
        assert_eq!(s.display_order(), vec![1, 0, 2]);

        // Recovered-but-recently-down is promoted too.
        let s = state(vec![
            monitor("a", Up, false),
            monitor("b", Up, true),
            monitor("c", Up, false),
        ]);
        assert_eq!(s.display_order(), vec![1, 0, 2]);
    }
}
