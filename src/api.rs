//! The public status-page JSON API.
//!
//! A public Uptime Kuma status page exposes exactly two unauthenticated
//! endpoints, which is all this program ever touches:
//!
//! * `/api/status-page/{slug}`            - title, groups, monitors, incidents
//! * `/api/status-page/heartbeat/{slug}`  - last ~50 beats and 24h uptime
//!
//! Unknown fields are ignored on purpose: Uptime Kuma versions differ and a
//! missing optional key must never be fatal.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

/// Origin plus slug, parsed from the status page URL the user passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusPage {
    pub origin: String,
    pub slug: String,
}

impl StatusPage {
    /// Accepts `https://host/status/slug` with an optional trailing slash,
    /// query string, or fragment. A sub-path deployment
    /// (`https://host/kuma/status/slug`) works too.
    pub fn parse(url: &str) -> Result<Self, String> {
        const BAD: &str = "URL must look like https://host/status/<slug>";

        let url = url.trim();
        let url = url.split(['?', '#']).next().unwrap_or_default();
        let url = url.trim_end_matches('/');

        let (origin, rest) = url.split_once("/status/").ok_or(BAD)?;
        if !(origin.starts_with("http://") || origin.starts_with("https://")) {
            return Err(BAD.into());
        }
        let slug = rest.split('/').next().unwrap_or_default();
        if slug.is_empty() || origin.split_once("://").is_none_or(|(_, h)| h.is_empty()) {
            return Err(BAD.into());
        }
        Ok(StatusPage {
            origin: origin.to_string(),
            slug: slug.to_string(),
        })
    }

    pub fn config_url(&self) -> String {
        format!("{}/api/status-page/{}", self.origin, self.slug)
    }

    pub fn heartbeat_url(&self) -> String {
        format!("{}/api/status-page/heartbeat/{}", self.origin, self.slug)
    }

    /// Origin without the scheme - for error messages.
    pub fn host(&self) -> &str {
        self.origin
            .split_once("://")
            .map_or(self.origin.as_str(), |(_, h)| h)
    }
}

#[derive(Debug, Deserialize)]
pub struct ConfigResponse {
    pub config: Config,
    #[serde(default)]
    pub incident: Option<Incident>,
    #[serde(rename = "maintenanceList", default)]
    pub maintenance_list: Vec<Maintenance>,
    #[serde(rename = "publicGroupList", default)]
    pub public_group_list: Vec<ApiGroup>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct Incident {
    pub title: String,
    /// `info` | `warning` | `danger` | `primary` | `light` | `dark`
    #[serde(default)]
    pub style: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Maintenance {
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct ApiGroup {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "monitorList", default)]
    pub monitor_list: Vec<ApiMonitor>,
}

#[derive(Debug, Deserialize)]
pub struct ApiMonitor {
    pub id: u64,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatResponse {
    /// Keyed by monitor id rendered as a string. Oldest beat first.
    #[serde(rename = "heartbeatList", default)]
    pub heartbeat_list: HashMap<String, Vec<Beat>>,
    /// Keyed by `"{id}_24"`, value is a ratio in `0.0..=1.0`.
    #[serde(rename = "uptimeList", default)]
    pub uptime_list: HashMap<String, f64>,
}

/// One heartbeat. `status`: 0 down, 1 up, 2 pending, 3 maintenance.
#[derive(Debug, Clone, Deserialize)]
pub struct Beat {
    pub status: u8,
    /// `"YYYY-MM-DD HH:MM:SS.mmm"`, UTC.
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub ping: Option<f64>,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .user_agent(concat!("uptime-kuma-status/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get<T: serde::de::DeserializeOwned>(url: &str, host: &str) -> Result<T, String> {
    agent()
        .get(url)
        .call()
        .map_err(|e| describe(&e, host))?
        .body_mut()
        .read_json::<T>()
        .map_err(|e| describe(&e, host))
}

/// Turns a transport error into something a person can act on.
///
/// The timeout case matters more than it looks: an Uptime Kuma instance does
/// not 404 an unknown status page, it simply never answers, so a mistyped slug
/// surfaces as a timeout rather than as "not found". Saying only "timeout"
/// would send someone hunting a network fault that is not there.
fn describe(e: &ureq::Error, host: &str) -> String {
    match e {
        ureq::Error::Timeout(_) => {
            format!("{host} never answered - is the status page slug right?")
        }
        ureq::Error::StatusCode(404) => format!("{host} has no such status page"),
        ureq::Error::StatusCode(code) => format!("{host} returned HTTP {code}"),
        ureq::Error::HostNotFound => format!("cannot resolve {host}"),
        ureq::Error::ConnectionFailed | ureq::Error::Io(_) => format!("cannot reach {host}"),
        ureq::Error::Json(_) => {
            format!("{host} did not return status-page data - is the slug right?")
        }
        ureq::Error::Tls(_) => format!("TLS failed talking to {host}"),
        other => format!("{host}: {other}"),
    }
}

pub fn fetch_config(p: &StatusPage) -> Result<ConfigResponse, String> {
    get(&p.config_url(), p.host())
}

pub fn fetch_heartbeat(p: &StatusPage) -> Result<HeartbeatResponse, String> {
    get(&p.heartbeat_url(), p.host())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_url() {
        let p = StatusPage::parse("https://status.example.com/status/mine").unwrap();
        assert_eq!(p.origin, "https://status.example.com");
        assert_eq!(p.slug, "mine");
        assert_eq!(
            p.config_url(),
            "https://status.example.com/api/status-page/mine"
        );
        assert_eq!(
            p.heartbeat_url(),
            "https://status.example.com/api/status-page/heartbeat/mine"
        );
        assert_eq!(p.host(), "status.example.com");
    }

    #[test]
    fn tolerates_trailing_slash_query_and_fragment() {
        for u in [
            "https://h/status/s/",
            "https://h/status/s?dark=1",
            "https://h/status/s#top",
            "  https://h/status/s  ",
        ] {
            assert_eq!(StatusPage::parse(u).unwrap().slug, "s", "failed on {u}");
        }
    }

    #[test]
    fn supports_a_sub_path_deployment() {
        let p = StatusPage::parse("https://h/kuma/status/s").unwrap();
        assert_eq!(p.origin, "https://h/kuma");
        assert_eq!(p.config_url(), "https://h/kuma/api/status-page/s");
    }

    /// Guards against schema drift: these fixtures are real responses from a
    /// live Uptime Kuma instance, trimmed. `maintenanceList` is absent from
    /// them on purpose - a missing optional key must never be fatal.
    #[test]
    fn deserializes_real_responses() {
        let cfg: ConfigResponse =
            serde_json::from_str(include_str!("../tests/fixtures/config.json")).unwrap();
        assert_eq!(cfg.config.title, "WittyOneOff");
        assert_eq!(cfg.public_group_list.len(), 2);
        assert_eq!(cfg.public_group_list[0].name, "Services");
        assert_eq!(cfg.public_group_list[0].monitor_list[0].name, "socktop.io");
        assert!(cfg.incident.is_none());
        assert!(cfg.maintenance_list.is_empty());

        let hb: HeartbeatResponse =
            serde_json::from_str(include_str!("../tests/fixtures/heartbeat.json")).unwrap();
        let beats = &hb.heartbeat_list["2"];
        assert_eq!(beats.len(), 5);
        assert_eq!(beats[0].status, 1);
        assert!(beats[0].ping.is_some());
        assert!(hb.uptime_list.contains_key("2_24"));
    }

    #[test]
    fn tolerates_a_stripped_down_payload() {
        // Only the keys this program truly requires.
        let cfg: ConfigResponse = serde_json::from_str(r#"{"config":{}}"#).unwrap();
        assert_eq!(cfg.config.title, "");
        assert!(cfg.public_group_list.is_empty());
        let hb: HeartbeatResponse = serde_json::from_str("{}").unwrap();
        assert!(hb.heartbeat_list.is_empty());
    }

    #[test]
    fn errors_say_what_to_do_about_them() {
        let h = "status.example.com";
        assert_eq!(
            describe(&ureq::Error::StatusCode(404), h),
            "status.example.com has no such status page"
        );
        assert_eq!(
            describe(&ureq::Error::StatusCode(503), h),
            "status.example.com returned HTTP 503"
        );
        assert_eq!(
            describe(&ureq::Error::HostNotFound, h),
            "cannot resolve status.example.com"
        );
        assert_eq!(
            describe(&ureq::Error::ConnectionFailed, h),
            "cannot reach status.example.com"
        );
        // The one that would otherwise send someone chasing a network fault.
        let timeout = describe(&ureq::Error::Timeout(ureq::Timeout::Global), h);
        assert!(timeout.contains("slug"), "{timeout}");
    }

    #[test]
    fn rejects_bad_urls() {
        for u in [
            "https://h/dashboard",
            "status.example.com/status/s",
            "https://h/status/",
            "https:///status/s",
            "",
        ] {
            assert!(StatusPage::parse(u).is_err(), "should have rejected {u}");
        }
    }
}
