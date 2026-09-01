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
//!
//! Both endpoints are keyed by the page's slug, so the first job is turning
//! whatever URL the user pasted into an origin plus a slug. See
//! [`StatusPage::resolve`] - the `/status/<slug>` path is a default, not a rule.

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
    /// Turns the URL the user pasted into an origin plus a slug.
    ///
    /// `/status/<slug>` is only Uptime Kuma's *default* path. A reverse proxy
    /// can serve the page from anywhere, and Uptime Kuma's own "entry page"
    /// setting puts it at the site root - `https://status.kuma.pet/` is a live
    /// example. So when the path carries no slug, ask the page itself: Uptime
    /// Kuma serves HTML containing
    ///
    /// ```text
    /// <link rel="manifest" href="/api/status-page/<slug>/manifest.json">
    /// ```
    ///
    /// which names both the slug and the API base. That costs one extra HTTP
    /// request, paid only on the URLs that need it.
    pub fn resolve(url: &str) -> Result<Self, String> {
        match Self::parse(url) {
            Ok(page) => Ok(page),
            Err(_) => Self::discover(url),
        }
    }

    /// Accepts `https://host/status/slug` with an optional trailing slash,
    /// query string, or fragment. A sub-path deployment
    /// (`https://host/kuma/status/slug`) works too.
    pub fn parse(url: &str) -> Result<Self, String> {
        const BAD: &str = "URL must look like https://host/status/<slug>";

        let url = trim_url(url);

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

    /// Reads the page's HTML and takes the slug from its manifest link.
    fn discover(url: &str) -> Result<Self, String> {
        let url = trim_url(url);
        let (root, path) =
            split_root(url).ok_or("URL must start with http:// or https:// and name a host")?;
        let host = root.split_once("://").map_or(root, |(_, h)| h);

        let html = fetch_html(url, host)?;
        let (base, slug) = manifest_href(&html)
            .and_then(split_manifest_href)
            .ok_or_else(|| {
                format!("{host} is not an Uptime Kuma status page - pass the /status/<slug> URL")
            })?;

        // A lone candidate needs no proving: leave it to the normal fetch
        // cycle, which reports trouble with the wording [`describe`] already
        // gives it. Only an actual ambiguity is worth a probe.
        let mut candidates = candidates(root, path, base, slug);
        if candidates.len() > 1
            && let Some(i) = candidates.iter().position(|c| fetch_config(c).is_ok())
        {
            return Ok(candidates.swap_remove(i));
        }
        Ok(candidates.remove(0))
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

/// Drops surrounding space, a query string, a fragment and a trailing slash.
fn trim_url(url: &str) -> &str {
    url.trim()
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .trim_end_matches('/')
}

/// Splits an http(s) URL into `("https://host", "/path")`. `None` unless the
/// scheme is http or https and the host is non-empty.
fn split_root(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let cut = rest.find('/').unwrap_or(rest.len());
    if rest[..cut].is_empty() {
        return None;
    }
    Some((&url[..scheme.len() + 3 + cut], &rest[cut..]))
}

/// Pulls the status-page manifest href out of a page's HTML.
///
/// Deliberately not an HTML parse. The one link that matters is machine
/// generated by Uptime Kuma and always reads
/// `href="<base>/api/status-page/<slug>/manifest.json"`, so finding that
/// quoted value is enough - and an HTML parser in the dependency list would
/// buy nothing for it.
fn manifest_href(html: &str) -> Option<&str> {
    const NEEDLE: &str = "/api/status-page/";

    let mut from = 0;
    while let Some(hit) = html[from..].find(NEEDLE) {
        let at = from + hit;
        // The quotes around the attribute value delimit the href.
        let end = html[at..].find(['"', '\'']).map(|i| at + i)?;
        from = end;
        if let Some(start) = html[..at].rfind(['"', '\''])
            && html[start + 1..end].ends_with("/manifest.json")
        {
            return Some(&html[start + 1..end]);
        }
    }
    None
}

/// Splits a manifest href into the API base it implies and the slug.
///
/// ```text
/// /api/status-page/mine/manifest.json        -> ("",             "mine")
/// /kuma/api/status-page/mine/manifest.json   -> ("/kuma",        "mine")
/// https://h/api/status-page/mine/manifest.json -> ("https://h",  "mine")
/// ```
fn split_manifest_href(href: &str) -> Option<(&str, &str)> {
    let (base, rest) = href.split_once("/api/status-page/")?;
    let slug = rest.strip_suffix("/manifest.json")?;
    if slug.is_empty() || slug.contains('/') {
        return None;
    }
    Some((base, slug))
}

/// Where the API might live, most likely first.
///
/// The href Uptime Kuma injects is root-relative (`/api/...`), which is right
/// when the page is served from the site root and wrong when a reverse proxy
/// has mounted it under a prefix. So try the prefix the user's own URL implies
/// before falling back to the bare origin. An absolute href needs no guessing.
fn candidates(root: &str, path: &str, base: &str, slug: &str) -> Vec<StatusPage> {
    let page = |origin: String| StatusPage {
        origin,
        slug: slug.to_string(),
    };
    if base.starts_with("http://") || base.starts_with("https://") {
        return vec![page(base.to_string())];
    }

    let mut out = Vec::with_capacity(2);
    if !path.is_empty() {
        out.push(page(format!("{root}{path}{base}")));
    }
    let bare = format!("{root}{base}");
    if out.first().is_none_or(|p| p.origin != bare) {
        out.push(page(bare));
    }
    out
}

/// What a URL can be judged on without touching the network, returning the
/// title to show until the page announces its own.
///
/// Split out from [`StatusPage::resolve`] so that a malformed URL stays a
/// usage error on stderr while the *network* half of resolving happens on the
/// fetch thread. It must not run before the terminal is up: `q` would do
/// nothing for as long as it took.
pub fn precheck(url: &str) -> Result<String, String> {
    if let Ok(page) = StatusPage::parse(url) {
        return Ok(page.slug);
    }
    let (root, _) = split_root(trim_url(url)).ok_or(
        "URL must look like https://host/status/<slug>, \
         or the URL your setup serves the page from",
    )?;
    Ok(root.split_once("://").map_or(root, |(_, h)| h).to_string())
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

/// Reads a page's HTML so [`StatusPage::discover`] can look at its head.
///
/// Capped, and lossy about encoding: only the machine-generated manifest link
/// matters, and a mis-pointed URL must not be able to eat memory or fail over
/// a stray byte further down the page.
fn fetch_html(url: &str, host: &str) -> Result<String, String> {
    agent()
        .get(url)
        .call()
        .map_err(|e| describe(&e, host))?
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .lossy_utf8(true)
        .read_to_string()
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

    /// The head Uptime Kuma actually serves for a page hosted at the site
    /// root, trimmed. Taken from status.kuma.pet, the upstream demo.
    const ROOT_HOSTED_HTML: &str = r#"<!DOCTYPE html><html lang="en"><head>
        <link rel="icon" href="/icon.svg">
        <link rel="manifest" href="/api/status-page/uptime-kuma/manifest.json">
        <title>Uptime Kuma Status</title>
        </head><body><div id="app"></div></body></html>"#;

    #[test]
    fn finds_the_manifest_link_in_real_html() {
        let href = manifest_href(ROOT_HOSTED_HTML).unwrap();
        assert_eq!(href, "/api/status-page/uptime-kuma/manifest.json");
        assert_eq!(split_manifest_href(href).unwrap(), ("", "uptime-kuma"));
    }

    #[test]
    fn ignores_html_with_no_status_page_manifest() {
        for html in [
            "<html><head><title>nginx</title></head></html>",
            // A manifest, but not a status page's.
            r#"<link rel="manifest" href="/manifest.json">"#,
            // The API path, but not as a manifest link.
            r#"<a href="/api/status-page/mine">json</a>"#,
            // A slug with a slash in it is not a slug.
            r#"<link rel="manifest" href="/api/status-page/a/b/manifest.json">"#,
        ] {
            let found = manifest_href(html).and_then(split_manifest_href);
            assert!(found.is_none(), "should have found nothing in {html}");
        }
    }

    #[test]
    fn reads_a_sub_path_manifest_href() {
        let html = r#"<link rel="manifest" href="/kuma/api/status-page/mine/manifest.json">"#;
        let (base, slug) = manifest_href(html).and_then(split_manifest_href).unwrap();
        assert_eq!((base, slug), ("/kuma", "mine"));
    }

    /// The whole point of the issue this fixes: a page at the site root.
    #[test]
    fn a_root_hosted_page_yields_one_candidate() {
        let c = candidates("https://status.kuma.pet", "", "", "uptime-kuma");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].origin, "https://status.kuma.pet");
        assert_eq!(
            c[0].config_url(),
            "https://status.kuma.pet/api/status-page/uptime-kuma"
        );
    }

    /// Uptime Kuma writes the href root-relative, which a reverse proxy that
    /// mounted it under a prefix makes wrong. Try the user's own prefix first.
    #[test]
    fn a_proxied_prefix_is_tried_before_the_bare_origin() {
        let c = candidates("https://h", "/kuma", "", "mine");
        let origins: Vec<&str> = c.iter().map(|p| p.origin.as_str()).collect();
        assert_eq!(origins, ["https://h/kuma", "https://h"]);
    }

    #[test]
    fn an_absolute_manifest_href_needs_no_guessing() {
        let c = candidates("https://h", "/anything", "https://kuma.internal", "mine");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].origin, "https://kuma.internal");
    }

    #[test]
    fn splits_http_urls_and_rejects_the_rest() {
        assert_eq!(split_root("https://h/kuma"), Some(("https://h", "/kuma")));
        assert_eq!(split_root("http://h:3001"), Some(("http://h:3001", "")));
        for u in ["ftp://h/x", "status.example.com", "https:///x", ""] {
            assert!(split_root(u).is_none(), "should have rejected {u}");
        }
    }

    /// `resolve` must not reach the network for a URL `parse` already handles.
    #[test]
    fn resolve_takes_the_offline_path_for_a_default_url() {
        let p = StatusPage::resolve("https://h/status/s/").unwrap();
        assert_eq!(p, StatusPage::parse("https://h/status/s").unwrap());
    }

    /// The offline half must stay offline: it runs before the terminal is up.
    #[test]
    fn precheck_names_a_title_without_the_network() {
        assert_eq!(precheck("https://h/status/mine/").unwrap(), "mine");
        assert_eq!(
            precheck("https://status.kuma.pet/").unwrap(),
            "status.kuma.pet"
        );
        assert_eq!(precheck("http://h:3001").unwrap(), "h:3001");
        for u in ["ftp://h/x", "status.example.com", ""] {
            assert!(precheck(u).is_err(), "should have rejected {u}");
        }
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
