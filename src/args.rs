//! Command-line arguments. Hand-rolled: four options do not justify a parser crate.

pub const HELP: &str = "\
uptime-kuma-status - render a public Uptime Kuma status page in the terminal

USAGE:
    uptime-kuma-status <STATUS_PAGE_URL> [OPTIONS]

ARGS:
    <STATUS_PAGE_URL>   e.g. https://status.example.com/status/myslug, or
                        whatever URL your setup serves the page from

OPTIONS:
    --interval <secs>   refresh period, default 60 (minimum 5)
    --max <n>           max monitors shown per screen (default: as many as fit)
    --ascii             ASCII-only glyphs for terminals with poor Unicode fonts
    -h, --help          print this help

KEYS:
    q / Esc / Ctrl-C    quit
    r                   refresh now
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub url: String,
    pub interval: u64,
    pub max: Option<usize>,
    pub ascii: bool,
}

/// `Ok(None)` means "help was requested"; the caller prints [`HELP`] and exits 0.
pub fn parse(argv: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut url: Option<String> = None;
    let mut interval: u64 = 60;
    let mut max: Option<usize> = None;
    let mut ascii = false;

    let mut it = argv.peekable();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(None),
            "--ascii" => ascii = true,
            "--interval" => {
                let v = it.next().ok_or("--interval needs a value")?;
                interval = v
                    .parse()
                    .map_err(|_| format!("--interval: not a number: {v}"))?;
                if interval < 5 {
                    return Err("--interval must be >= 5 seconds".into());
                }
            }
            "--max" => {
                let v = it.next().ok_or("--max needs a value")?;
                let n: usize = v.parse().map_err(|_| format!("--max: not a number: {v}"))?;
                if n == 0 {
                    return Err("--max must be >= 1".into());
                }
                max = Some(n);
            }
            other if other.starts_with('-') => return Err(format!("unknown flag: {other}")),
            other => {
                if url.is_some() {
                    return Err(format!("unexpected extra argument: {other}"));
                }
                url = Some(other.to_string());
            }
        }
    }

    let url = url.ok_or("missing <STATUS_PAGE_URL>")?;
    Ok(Some(Args {
        url,
        interval,
        max,
        ascii,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Result<Option<Args>, String> {
        parse(a.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_args_is_an_error() {
        assert!(p(&[]).is_err());
    }

    #[test]
    fn help_is_not_an_error() {
        assert_eq!(p(&["--help"]), Ok(None));
        assert_eq!(p(&["-h"]), Ok(None));
    }

    #[test]
    fn defaults() {
        let a = p(&["https://h/status/s"]).unwrap().unwrap();
        assert_eq!(a.interval, 60);
        assert_eq!(a.max, None);
        assert!(!a.ascii);
    }

    #[test]
    fn all_options() {
        let a = p(&[
            "--max",
            "5",
            "--ascii",
            "https://h/status/s",
            "--interval",
            "30",
        ])
        .unwrap()
        .unwrap();
        assert_eq!(a.url, "https://h/status/s");
        assert_eq!(a.interval, 30);
        assert_eq!(a.max, Some(5));
        assert!(a.ascii);
    }

    #[test]
    fn interval_floor_is_enforced() {
        assert!(p(&["https://h/status/s", "--interval", "3"]).is_err());
        assert!(p(&["https://h/status/s", "--interval", "x"]).is_err());
    }

    #[test]
    fn unknown_flag_and_extra_arg_are_errors() {
        assert!(p(&["https://h/status/s", "--nope"]).is_err());
        assert!(p(&["a", "b"]).is_err());
    }
}
