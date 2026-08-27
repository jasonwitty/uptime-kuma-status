# uptime-kuma-status — Plan

A tiny ratatui TUI that renders a **public Uptime Kuma status page** in the
terminal. Built to sit in a tmux pane inside
[socktop-swipe](https://gt.wittyoneoff.com/jason/socktop-swipe), next to socktop.

## Scope (fixed)

- Display one public Uptime Kuma status page. That's it.
- No authentication, no socket.io, no management, no dashboards, no other
  monitoring systems. Ever.
- Read-only, poll-based, no persistent state or config files.
- Code should read like a ratatui example project: one crate, a handful of
  short files, minimal dependencies.

## Usage

```
uptime-kuma-status <STATUS_PAGE_URL> [--interval <secs>] [--max <n>]

  <STATUS_PAGE_URL>   e.g. https://status.wittyoneoff.com/status/wittyoneoff
  --interval <secs>   refresh period, default 60 (same as the web page)
  --max <n>           max monitors shown per screen (default: as many as fit)
  --ascii             ASCII-only glyphs for terminals with poor Unicode fonts
  -h, --help          print this help
```

Running with no URL prints the same help and exits non-zero. No env vars.

## Data source

The Python `uptime-kuma-api` library is **not** relevant — it wraps the
authenticated socket.io API. Public status pages expose two plain JSON
endpoints, no auth, which the web page itself polls. Verified against the
real page:

| Endpoint | Gives us |
| --- | --- |
| `GET {origin}/api/status-page/{slug}` | `config` (title, description, footerText), `incident`, `maintenanceList`, `publicGroupList[] → monitorList[] {id, name, type}` |
| `GET {origin}/api/status-page/heartbeat/{slug}` | `heartbeatList[id][]` (last 50: `status`, `time`, `ping`, `msg`), `uptimeList["{id}_24"]` (24h ratio) |

`status`: 0 down · 1 up · 2 pending · 3 maintenance.

URL handling: from `https://host/status/<slug>` take origin + slug and build
the two API URLs. Fetch the config endpoint once at startup (and again on
manual refresh); poll the heartbeat endpoint every `--interval` seconds.

## Architecture

```
src/
  main.rs     arg parsing (hand-rolled, ~40 lines), terminal setup, event loop
  api.rs      two serde structs + one fetch() using ureq
  model.rs    flatten API JSON into Vec<Monitor> + overall status, sort rules
  ui.rs       draw(frame, &state): layout breakpoints + widgets
```

Roughly 400–600 lines total.

**Concurrency:** one `std::thread` does the HTTP fetch and sends results over an
`mpsc` channel; the main thread runs the crossterm event loop with a 250 ms
poll tick. No tokio. Rationale: we make one HTTP request per minute — an async
runtime would add a dependency, a macro, and `.await` noise for zero visible
benefit. (Easy to swap later; the fetch is isolated in one function.)

**Dependencies (5):**

| Crate | Why |
| --- | --- |
| `ratatui` 0.30 | widgets/layout (matches socktop) |
| `crossterm` 0.29 | terminal backend + events (matches socktop) |
| `ureq` (rustls feature) | blocking HTTPS client, small |

No `chrono`: the only time shown is "updated Ns ago", and the 10-minute
"recently down" window is computed relative to the newest heartbeat with a
20-line hand-written timestamp parser.
| `serde` + `serde_json` | JSON → structs |

No `clap`, no `anyhow`, no `tokio`, no `chrono` (timestamps only need string
slicing for `HH:MM`).

## Theming

Named `ratatui::style::Color` only — never `Color::Rgb` — so the user's terminal
palette is the theme (same rule as socktop):

| Meaning | Color |
| --- | --- |
| up | `Green` |
| down | `Red` |
| pending | `Yellow` |
| maintenance | `Blue` |
| no data | `DarkGray` |
| text / borders | default fg, `Gray` for muted |
| title | default fg, bold |

## Portability

This is a general-purpose tool first. The socktop-swipe panel (1024×600,
~100–130 cols × 30–40 rows full-screen, ~50×15 tiled) is the **tested
reference target**, not a hardcoded assumption:

- Never assume a terminal size. Every layout decision is derived from
  `frame.area()` on every draw.
- Must be pleasant in an ordinary 80×24 terminal and a 200×50 one.
- Must work with a 16-color palette; 256/true-color is not required.
- `--ascii` flag swaps every Unicode glyph for an ASCII fallback (markers,
  heartbeat levels, status glyphs) for terminals with poor fonts.
- Plain alternate-screen TUI over stdin/stdout — no tmux/X/Wayland assumptions,
  so it also works inside a quickshell/waybar popup terminal later. A future
  `--once` flag (render one frame to stdout, exit) is plausible for status-bar
  use; keep `ui::draw(&State, Frame)` pure so that's a small addition, but do
  **not** build it in v1.

## Layout

Design goal: **everything visible, no scrolling**. Stack side by side freely.
Breakpoints must be usable from ~50×15 up to anything.

### Fixed chrome (3 rows + optional banners)

```
┌ WittyOneOff ───────────────────────────────── updated 20:31 ┐   title row
│ ✔ All Systems Operational                                    │   overall status
│ ▲ Incident: <title>  ·  🛠 Maintenance: <title>              │   only if present
│   ... monitors ...                                            │
│ ■ Services  ■ Kuberneties  ■ GlusterFS Volume    +3 more ─ q │   legend / footer
└──────────────────────────────────────────────────────────────┘
```

Overall status text/glyph follows the web page:

| State | Glyph + text |
| --- | --- |
| all up | `✔ All Systems Operational` (green) |
| some down | `✘ Partially Degraded Service` (red — wall display must be glanceable) |
| all down | `✘ Degraded Service` (red) |
| some pending, none down | `… Some Checks Pending` (yellow) |
| any maintenance | `🛠 Under Maintenance` (blue) |
| nothing fetched yet / error | `? Connecting…` / `✘ Cannot reach <host>` (gray/red) |

### Monitor row — combined heartbeat + latency bar (recommended)

Answer to "is latency a pipe dream": no, if each heartbeat cell does double
duty — **color = status, height = ping**. Uses the block chars
`▁▂▃▄▅▆▇█` (like ratatui `Sparkline`). Down beats render as a full-height red
block so they can't be mistaken for a fast up beat. Costs zero extra columns
versus a plain heartbeat bar.

```
 ● socktop.io            98.65%  ▂▂▃▂▂▆▂▃▄▂▂▂▂▃▂▂▂▂▂▂▂▂▂▂█▂▂▃▂▂▂▂▃▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂   72ms
   ^ status dot   name   24h up  50 beats, green/red/yellow/blue, height=ping   latest ping
```

Height scale: per-monitor, latest-50 max → 8 levels (so a 70 ms HTTP check and
a 2 ms ICMP ping both use the full range). Ping missing (`null`) → `▁`.

Column widths: dot 2 · name 20 (truncated with `…`) · uptime 8 · bar N · ping 7.

### Breakpoints (per monitor row, by available width)

| Width available | Row shape |
| --- | --- |
| ≥ 90 | `● name(20) 98.65% [50 beats] 72ms` |
| 60–89 | `● name(16) 98.6% [last 24–40 beats] 72ms` |
| 40–59 | `● name(14) 98% [last 10–20 beats]` |
| < 40 | `● name(…)  98%` |

Bar length = whatever is left after the fixed columns, capped at 50, minimum 10
before dropping the bar entirely.

### Fitting rows (by available height)

Available rows = height − chrome. Monitors flow into **columns** side by side
when they don't fit in one column and the width allows ≥ 2 columns of the
`40–59` shape or better. Beyond that:

1. Show the first N that fit (or `--max`, whichever is smaller).
2. Footer shows `+K more`.
3. **Priority override:** monitors that are currently down, pending, or were
   down at any point in the last 10 minutes are sorted first and always shown;
   healthy monitors get hidden instead. If *everything* is down (or nothing
   is), fall back to the page's natural order.

Grouping is low priority: rows are flat. Each row's status dot is prefixed by
a **group marker** — a shape × color pair — and the footer legend maps marker →
group name. If there is only one group, marker and legend are omitted.

Shapes (single-cell, widely available in terminal fonts, in this order):

```
■  ◆  ▲  ●  ▼  ◀  ▶  ★  ⬢  ◼  ◇  △  ○  ▽  ◁  ▷  ☆  ⬡
```

Colors, in this order: `Cyan Magenta Blue Yellow Green Red White`.

Assignment: group *i* gets `SHAPES[i % 18]` with `COLORS[i / 18 % 7]` — i.e.
walk all shapes in the first color before moving to the next, which keeps the
first several groups maximally distinct (different shape *and* we never reuse
a shape until 18 groups). 18 × 7 = 126 unique markers, far beyond any sane
page. The status dot itself stays `●` so the group marker is never confused
with status (`■● name`, `◆● name`, `▲● name` …).

Legend example: `■ Services   ◆ Kuberneties   ▲ GlusterFS Volume`.

### Mockup — full screen (~120×36)

```
┌ WittyOneOff ────────────────────────────────────────────────────────────────────────── updated 20:31:42 ┐
│                                                                                                          │
│  ✔ All Systems Operational                                                                              │
│                                                                                                          │
│  ■● socktop.io           98.65%  ▂▂▃▂▂▆▂▃▄▂▂▂▂▃▂▂▂▂▂▂▂▂▂▂█▂▂▃▂▂▂▂▃▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂   72ms              │
│  ■● Nginx Proxy Manager  99.93%  ▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂▂   14ms              │
│  ■● unami                99.79%  ▃▃▃▃▄▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃▃   88ms              │
│  ... (19 rows total)                                                                                     │
│                                                                                                          │
│  ■ Services   ◆ Kuberneties   ▲ GlusterFS Volume                                              q quit    │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### Mockup — tiled overview pane (~50×15)

```
┌ WittyOneOff ────────────── 20:31 ┐
│ ✔ All Systems Operational        │
│ ■● socktop.io      98% ▂▂▃▂▆▂▃▂▂ │
│ ■● Nginx Proxy M…  99% ▂▂▂▂▂▂▂▂▂ │
│ ■● unami           99% ▃▃▃▄▃▃▃▃▃ │
│ ■● nextcloud       99% ▂▂▂▂▂▂▂▂▂ │
│ ■● Git Tea         99% ▂▂▂▃▂▂▂▂▂ │
│ ■● Casa OS        100% ▂▂▂▂▂▂▂▂▂ │
│ ■● Home Assistant  99% ▂▂▂▂▂▂▂▂▂ │
│ ■● tududi          99% ▂▂▂▂▂▂▂▂▂ │
│ ◆● K3s - Control… 100% ▁▁▁▁▁▁▁▁▁ │
│ ◆● K3s Worker 1   100% ▁▁▁▁▁▁▁▁▁ │
│ ■ Services ◆ Kube… ▲ Glu… +8 more │
└──────────────────────────────────┘
```

If a Gluster brick went down it would jump to the top of this list and push a
healthy Service row into the `+N more`.

## Behaviour

- Resize (`SIGWINCH` from `tmux resize-pane -Z`) → relayout on next frame; no
  refetch.
- Keys: `q` / `Esc` / `Ctrl-C` quit; `r` force refresh. Nothing else.
- Title right side shows `updated Ns ago` (no clock time → no timezone code).
- Fetch failure: keep showing last good data, overall status row turns to
  `✘ Cannot reach <host> (retrying)` in red, title shows last successful
  update time. First-fetch failure shows the same message on an otherwise
  empty screen.
- No mouse, no scrolling (touch panel only swipes, which tmux handles).

## Out of scope for v1 (possible later)

- Incident/maintenance body text (v1 shows title only, one line each).
- Certificate expiry, tags, custom footer text.
- Multiple status pages in one instance (run two panes instead).
- Packaging (crates.io / deb) — after it works in the rack.

## Implementation guide

Written so an implementer can build this without re-deriving decisions. Keep
functions small and pure; all sizing math lives in `layout.rs` and is
unit-tested without a terminal.

### Cargo.toml

```toml
[package]
name = "uptime-kuma-status"
version = "0.1.0"
edition = "2024"
description = "Render a public Uptime Kuma status page in the terminal"
license = "MIT"

[dependencies]
ratatui = "0.30"
crossterm = "0.29"
ureq = { version = "3", features = ["rustls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

Verified 2026-08-26: ureq 3.4, ratatui 0.30.2, crossterm 0.29. ureq 3.x API:
`ureq::get(url).call()?.body_mut().read_json::<T>()?`. 2.x:
`ureq::get(url).call()?.into_json::<T>()?`. Either is fine; set a 10 s
timeout.

### Files and responsibilities

```
src/main.rs    parse args → run() → always restore terminal
src/args.rs    Args struct, parse(), HELP text
src/api.rs     StatusPage (origin+slug), serde structs, fetch_config/fetch_heartbeat
src/model.rs   Status, Monitor, State, Overall, build_monitors(), overall(), display_order()
src/layout.rs  pure math: RowShape, Plan, plan(), row_shape()
src/ui.rs      draw(frame, &State, &Plan, &Args) — widgets only, no math
src/glyphs.rs  const tables + Glyphs { unicode | ascii } accessor
```

### args.rs

```rust
pub struct Args { pub url: String, pub interval: u64, pub max: Option<usize>, pub ascii: bool }
pub fn parse(argv: impl Iterator<Item = String>) -> Result<Args, String>
pub const HELP: &str = "...";   // exactly the Usage block above
```

- `-h`/`--help` → print HELP to stdout, exit 0.
- No URL → print HELP to stderr, exit 2.
- `--interval` < 5 → `Err("interval must be >= 5 seconds")`.
- Unknown flag → `Err("unknown flag: ...")`; main prints error + HELP, exit 2.

### api.rs

```rust
pub struct StatusPage { pub origin: String, pub slug: String }
impl StatusPage {
    /// "https://h/status/slug", "https://h/status/slug/", "https://h/status/slug?x" all → (https://h, slug)
    pub fn parse(url: &str) -> Result<Self, String>   // Err: "URL must look like https://host/status/<slug>"
    pub fn config_url(&self) -> String     // {origin}/api/status-page/{slug}
    pub fn heartbeat_url(&self) -> String  // {origin}/api/status-page/heartbeat/{slug}
    pub fn host(&self) -> &str             // origin without scheme, for error messages
}

#[derive(Deserialize)] pub struct ConfigResponse {
    pub config: Config,
    #[serde(default)] pub incident: Option<Incident>,
    #[serde(rename = "maintenanceList", default)] pub maintenance_list: Vec<Maintenance>,
    #[serde(rename = "publicGroupList", default)] pub public_group_list: Vec<ApiGroup>,
}
#[derive(Deserialize)] pub struct Config { pub title: String, #[serde(default)] pub description: Option<String> }
#[derive(Deserialize)] pub struct Incident { pub title: String, #[serde(default)] pub style: Option<String> } // info|warning|danger|primary|light|dark
#[derive(Deserialize)] pub struct Maintenance { pub title: String }
#[derive(Deserialize)] pub struct ApiGroup { pub name: String, #[serde(rename = "monitorList", default)] pub monitor_list: Vec<ApiMonitor> }
#[derive(Deserialize)] pub struct ApiMonitor { pub id: u64, pub name: String }

#[derive(Deserialize)] pub struct HeartbeatResponse {
    #[serde(rename = "heartbeatList", default)] pub heartbeat_list: HashMap<String, Vec<Beat>>, // key: id as string
    #[serde(rename = "uptimeList",    default)] pub uptime_list:    HashMap<String, f64>,      // key: "{id}_24"
}
#[derive(Deserialize, Clone)] pub struct Beat { pub status: u8, pub time: String, #[serde(default)] pub ping: Option<f64> }

pub fn fetch_config(p: &StatusPage) -> Result<ConfigResponse, String>
pub fn fetch_heartbeat(p: &StatusPage) -> Result<HeartbeatResponse, String>
```

Unknown JSON fields are ignored by default — do not add `deny_unknown_fields`.
Uptime Kuma versions differ; never fail on a missing optional key. `time` is
`"YYYY-MM-DD HH:MM:SS.mmm"`, UTC, oldest first.

### model.rs

```rust
#[derive(Copy, Clone, PartialEq, Eq, Debug)] pub enum Status { Up, Down, Pending, Maintenance, Unknown }
impl Status {
    pub fn from_code(c: u8) -> Self   // 0 Down, 1 Up, 2 Pending, 3 Maintenance, _ Unknown
    pub fn color(self) -> Color       // Green, Red, Yellow, Blue, DarkGray
}

pub struct Monitor {
    pub id: u64, pub name: String, pub group: usize,   // index into State.groups
    pub beats: Vec<Beat>,                                // oldest..newest, ≤ 50
    pub uptime24: Option<f64>, pub current: Status,
    pub latest_ping: Option<f64>, pub recently_down: bool,
}

#[derive(Copy, Clone, PartialEq, Eq)] pub enum Overall { Operational, PartiallyDegraded, Degraded, Maintenance, Pending, Unknown }

pub struct State {
    pub title: String, pub groups: Vec<String>, pub monitors: Vec<Monitor>,  // page order
    pub incident: Option<(String, Color)>, pub maintenance: Vec<String>,
    pub last_ok: Option<Instant>, pub error: Option<String>, pub host: String,
}
impl State {
    pub fn apply(&mut self, msg: Msg)             // Data → rebuild + last_ok=now + error=None; Error → error=Some
    pub fn overall(&self) -> Overall
    pub fn display_order(&self) -> Vec<usize>      // indices into monitors
}

pub fn build_monitors(cfg: &ConfigResponse, hb: &HeartbeatResponse) -> Vec<Monitor>
pub fn parse_ts(s: &str) -> Option<i64>           // "YYYY-MM-DD HH:MM:SS[.mmm]" → seconds (days-from-civil, ~20 lines)
```

Rules:

- `current` = status of the **last** beat, `Unknown` if none.
- `latest_ping` = `ping` of the last beat.
- `recently_down`: let `newest = parse_ts(last beat of this monitor)`. True if
  any beat has `status == 0` and `newest - parse_ts(beat.time) <= 600`. Uses
  the beat clock, not the system clock, so timezones never matter.
- `overall()`: n_down == n (n > 0) → `Degraded`; n_down > 0 →
  `PartiallyDegraded`; any Maintenance → `Maintenance`; any Pending →
  `Pending`; n > 0 → `Operational`; else `Unknown`.
- `display_order()`: `hot(m) = m.current != Up || m.recently_down`. If every
  monitor is hot **or** none is hot → `0..n`. Else stable partition: hot
  monitors in page order, then the rest in page order.
- Incident color from `style`: `danger` → Red, `warning` → Yellow, else Blue.

### glyphs.rs

```rust
pub const SHAPES:  [&str; 18] = ["■","◆","▲","●","▼","◀","▶","★","⬢","◼","◇","△","○","▽","◁","▷","☆","⬡"];
pub const SHAPES_ASCII: [&str; 18] = ["#","+","^","o","v","<",">","*","@","%","=","~","O","V","{","}","&","0"];
pub const MARKER_COLORS: [Color; 7] = [Cyan, Magenta, Blue, Yellow, Green, Red, White];
pub const BEAT_LEVELS: [char; 8] = ['▁','▂','▃','▄','▅','▆','▇','█'];
pub const BEAT_LEVELS_ASCII: [char; 8] = ['.',':','-','=','+','*','#','%'];
pub fn marker(group_idx: usize, ascii: bool) -> (&'static str, Color)  // SHAPES[i % 18], COLORS[(i / 18) % 7]
pub fn dot(ascii) -> &str          // "●" / "o"
pub fn ok(ascii) / bad(ascii) / maint(ascii) / pending(ascii) -> &str   // "✔"/"OK", "✘"/"!!", "⚙"/"MT", "…"/".."
```

`⚙` (U+2699) is single-width in most fonts; do **not** use the 🛠 emoji (double
width breaks alignment).

### layout.rs (pure, unit-tested)

```rust
pub struct RowShape { pub name_w: u16, pub uptime_w: u16, pub bar_w: u16, pub ping_w: u16 }
pub struct Plan { pub shape: RowShape, pub columns: u16, pub rows_per_col: u16, pub shown: usize, pub hidden: usize }

pub fn row_shape(col_w: u16, has_marker: bool) -> RowShape
pub fn plan(width: u16, height: u16, n: usize, has_marker: bool, max: Option<usize>, banner_rows: u16) -> Plan
```

`row_shape`: `fixed = 1 (left pad) + (has_marker as u16) + 2 ("● ") + 1 (right pad)`;
`rem = col_w.saturating_sub(fixed)`. Columns are separated by single spaces
(count them in `fixed` as +1 per non-zero column after the name).

| `col_w` | name_w | uptime_w | bar_w | ping_w |
| --- | --- | --- | --- | --- |
| ≥ 90 | 20 | 7 (`98.65%`) | min(50, leftover) | 7 (`  72ms`) |
| 60–89 | 16 | 6 (`98.6%`) | leftover, if ≥ 10 else 0 | 7 if bar_w ≥ 24 else 0 |
| 40–59 | 14 | 4 (`98%`) | leftover if ≥ 10 else 0 | 0 |
| < 40 | leftover − 5 (min 6) | 4 | 0 | 0 |

"leftover" = `rem − name_w − uptime_w − ping_w − separators`. Compute in
`u16` with `saturating_sub` everywhere.

`plan`:

1. `chrome = 2 (borders) + 1 (overall line) + banner_rows`. Legend lives in
   the bottom border (`title_bottom`) so it costs 0 rows.
   `avail = height.saturating_sub(chrome)`. `avail == 0` → `shown = 0`.
2. `columns = 1`; while `columns * avail < n` and `row_shape(width / (columns+1), ..).bar_w >= 10`:
   `columns += 1`. (Adding a column is only allowed while every column still
   fits a ≥10-beat bar; cap at 6 columns.)
3. `capacity = columns * avail`; `shown = min(n, capacity, max.unwrap_or(n))`;
   `hidden = n − shown`; `rows_per_col = ceil(shown / columns)`;
   `shape = row_shape(width / columns, has_marker)`.

### ui.rs

`pub fn draw(f: &mut Frame, s: &State, args: &Args)` computes `banner_rows`
(incident 1 + maintenance 1 if any), calls `layout::plan`, then renders:

1. **Outer block**: `Borders::ALL`, `title(" {title} ")` top-left,
   `title_top(Line::from(" updated {N}s ago ").right_aligned())` (or `" no data "`),
   `title_bottom` = legend (left) and `" +{hidden} more  q quit "` (right). Legend
   only when `groups.len() > 1`: spans `{marker} {group}` in marker color, two
   spaces apart, truncated with `…` if it would collide with the right text.
2. **Overall line** (row 0 inside): bold, colored:
   `Operational` → `ok` green "All Systems Operational";
   `PartiallyDegraded` → `bad` red "Partially Degraded Service";
   `Degraded` → `bad` red "Degraded Service";
   `Maintenance` → `maint` blue "Under Maintenance";
   `Pending` → `pending` yellow "Some Checks Pending";
   `Unknown`/no data → gray "Connecting…".
   If `s.error.is_some()` **override** with red `bad` "Cannot reach {host} (retrying)".
3. **Banners**: `▲ Incident: {title}` in incident color; `⚙ Maintenance: {t1}; {t2}` blue.
4. **Grid**: `Layout::horizontal(vec![Constraint::Fill(1); columns])` over the
   remaining area. Monitor `k` (from `display_order()[..shown]`) goes to column
   `k / rows_per_col`, row `k % rows_per_col`. Each row is one `Line` of spans:
   `[" "] [marker] ["● " status color] [name padded/truncated to name_w] [" "] [uptime right-aligned uptime_w] [" "] [bar] [" "] [ping]`.
   Skip a span entirely when its width is 0.
5. **Bar**: take the last `bar_w` beats. If fewer, left-pad with `▁` in
   DarkGray. `max_ping = max(ping over these beats).max(1.0)`.
   Per beat: `level = ((ping / max_ping) * 7.0).round() as usize` clamped to
   0..=7, `None` → 0. Status Down → level 7 + Red; Pending → Yellow;
   Maintenance → Blue; Up → Green. Char = `BEAT_LEVELS[level]`.
6. **Uptime**: `uptime_w` 7 → `{:.2}%`, 6 → `{:.1}%`, 4 → `{:.0}%`;
   `None` → `--`. Right-align. (`100%` fits width 4 exactly; `100.0%`/`100.00%` fit 6/7.)
7. **Ping**: `format!("{:>5}ms", p.round() as i64)`; `None` → 7 spaces.
8. **Name**: truncate by `chars().count()`; if longer than `name_w`, take
   `name_w − 1` chars + `…`. Pad with spaces to `name_w`.

### main.rs

```rust
enum Msg { Data(ConfigResponse, HeartbeatResponse), Error(String) }

fn main() -> ExitCode
fn run(args: Args) -> Result<(), String> {
    let page = StatusPage::parse(&args.url)?;
    let (tx, rx) = mpsc::channel::<Msg>();
    let refresh = Arc::new(AtomicBool::new(false));
    // fetcher thread
    thread::spawn(move || loop {
        let msg = match (fetch_config(&page), fetch_heartbeat(&page)) {
            (Ok(c), Ok(h)) => Msg::Data(c, h), (Err(e), _) | (_, Err(e)) => Msg::Error(e) };
        if tx.send(msg).is_err() { return; }
        // sleep `interval` seconds in 250 ms slices, break early if refresh flag set
    });
    let mut terminal = ratatui::init();        // raw mode + alt screen + panic hook that restores
    let mut state = State::new(title placeholder = page.slug, host = page.host());
    loop {
        while let Ok(m) = rx.try_recv() { state.apply(m); }
        terminal.draw(|f| ui::draw(f, &state, &args)).map_err(|e| e.to_string())?;
        if event::poll(Duration::from_millis(250)).map_err(..)? {
            match event::read() { Key(q | Esc | Ctrl-C) => break, Key(r) => refresh.store(true), _ => {} }
        }
    }
    ratatui::restore();
    Ok(())
}
```

- Fetch **both** endpoints every cycle so a newly added monitor appears
  without restart. Config is a few KB; cost is irrelevant.
- Only react to `KeyEventKind::Press` (Windows sends Release too).
- On `Msg::Error` keep old monitors; UI shows the red error line and
  `updated Ns ago` keeps counting from `last_ok`.

### Tests (`#[cfg(test)]` in each module)

- `args::parse`: no args → Err; `--interval 3` → Err; `--max 5 --ascii URL` ok.
- `StatusPage::parse`: trailing slash, query string, missing `/status/` → Err.
- `layout::plan`: `(120,36,19,true,None,0)` → 1 column, shown 19, bar_w 50;
  `(50,15,19,true,None,0)` → 1 column, shown 12, hidden 7; `(200,12,40,true,None,0)`
  → ≥ 2 columns; `(30,10,5,false,None,0)` → bar_w 0, shown 5; `(80,3,5,..)` → shown 0.
- `model`: `parse_ts("2026-08-26 20:08:39.799")` matches a hand-computed
  value; `display_order` cases: all up → identity, one down → first, all down
  → identity, recently_down promotes; `overall` for each variant.
- Serde: `tests/fixtures/config.json` + `heartbeat.json` captured from the
  real page (strip `customCSS`, trim heartbeats to ~5 per monitor) deserialize.

### Definition of done (v1)

- `cargo build` warning-free, `cargo clippy` clean, `cargo test` green.
- Works against `https://status.wittyoneoff.com/status/wittyoneoff` and one
  other public Uptime Kuma page, at 80×24, 120×36, 50×15, and 200×50.
- Resizing continuously never panics (no unchecked `u16` subtraction).
- Network unplugged → red error line, `q` still quits instantly.
- `--ascii` renders without any non-ASCII byte.
- README: usage, screenshot, and a tmux/socktop-swipe snippet.

## Findings from milestone 1 (verified against the live page)

**An unknown slug hangs the server for 90 s, then 504s.** It does not 404.
Verified with curl against `.../api/status-page/doesnotexist`. The 10 s client
timeout is therefore load-bearing, not decoration - without it a typo'd slug
freezes the UI for a minute and a half. Milestone 4 should map a timeout to a
friendlier line than ureq's `timeout: global`, e.g.
`Cannot reach {host} - is the slug right?`.

**The bar's ping scale needs fixing (ACCEPTED 2026-08-26, implemented in milestone 3).** The
spec above says "per-monitor, latest-50 max -> 8 levels". On real data that
reads badly: one latency spike compresses every other beat into the middle, so
a bar of `dashwise` (min 191 ms, p95 288 ms, max 1128 ms) renders as a flat run
of the same block with a single full-height spike, and `nextcloud` (a stable
731-1069 ms) sits permanently at 6-7 of 8, reading as "high" when it is simply
its own baseline. Naive min..max scaling has the same outlier problem.

What was verified to work, on all 19 real monitors:

```
lo = min(pings)
hi = max(p95(pings), lo + max(2.0, lo * 0.25))   // floor the range
level = clamp(round((ping - lo) / (hi - lo) * 7), 0, 7)
```

The p95 cap stops one spike from flattening the bar (the spike still saturates
at level 7); the floored range stops a genuinely flat monitor from being
amplified into fake noise - the K3s nodes ping 0-1 ms and correctly stay a calm
low line, where an unfloored min..max turned that same jitter into dramatic
peaks. Down beats still force level 7 + red regardless.

## Milestones

All four are complete as of 2026-08-26.

1. **Skeleton** (done) - deps, arg parsing + help, both endpoints fetched, model
   printed to stdout. Verified the data contract against the live page.
2. **Static render** (done) - ratatui loop, chrome, flat monitor rows, quit keys.
3. **Breakpoints** (done) - `layout.rs`, ping-as-height bar, multi-column flow,
   `+N more`, priority sort, adaptive row spacing.
4. **Polish** (done) - legend, incident/maintenance banners, actionable error
   states, `--max`, `--interval`, `--ascii` (including an ASCII border), README.

Not yet done: running it on the rack panel itself inside socktop-swipe. Every
size it will meet there has been exercised under tmux, but the panel is the
only test that counts for the panel.

## Decisions log

- 2026-08-26: group markers are shape × color (18 shapes × 7 named colors).
- 2026-08-26: overall status is **red whenever anything is down**, not yellow.
- 2026-08-26: no tokio; std::thread + mpsc for the fetch.
- 2026-08-26: latency encoded as beat height in the heartbeat bar.
- 2026-08-26: general-purpose tool; the 1024×600 rack panel is the tested
  reference, not an assumption. `--ascii` fallback; `--once` deferred.
- 2026-08-26: "updated Ns ago" instead of clock time — avoids a timezone crate.
- 2026-08-26: refetch config every cycle (not once) so added monitors appear.
- 2026-08-26: bar scale is floored-p95, not 0..max - see the milestone 1
  findings above for the formula and the evidence.
- 2026-08-26: dropped `Monitor::id`; nothing reads it (heartbeat lookup uses
  `ApiMonitor::id`), and an unused field is worse than the deviation.
- 2026-08-26 (m3): uptime is floored, never rounded up - `{:.0}` turned a real
  99.93% into "100%", which on a status page claims downtime never happened.
- 2026-08-26 (m3): column count is computed from `min(n, --max)`, so an
  explicit cap does not spread a handful of rows over needlessly thin columns.
- 2026-08-26 (m3): the bar rescales to the beats actually on screen, so a
  narrow pane scales to what it shows rather than to history it cropped.
- 2026-08-26 (m3): rows are separated by a blank line, and a blank line sits
  under the status line. Full-height blocks in adjacent rows otherwise touch
  and read as one solid mass. The spacing is adaptive: it is surrendered (row
  gap first, header gap second) before any monitor is hidden, and it may flow
  the rows into an extra column to keep everything visible.
- 2026-08-26 (m4): `--ascii` also swaps the window border for `+ - |`. A font
  that cannot draw block characters cannot draw box-drawing ones either, so a
  Unicode border defeated the point of the flag.
- 2026-08-26 (m4): fetch errors are described in terms the user can act on and
  shown verbatim in the status line, rather than a generic "cannot reach". A
  mistyped slug times out rather than 404ing, so it now says so explicitly.
- 2026-08-26 (m4): dropped `State::host`; the error strings carry the host.
- 2026-08-26 (m3): the degradation ladder was re-ordered. Name, uptime and
  ping are load-bearing and kept while they fit; the BAR is the elastic column
  - it shrinks as the column narrows and is dropped first. Previously ping was
  dropped to protect the bar, which lost the latency figure exactly when the
  bar was too short to convey latency itself. Ping is also compact now
  (`206ms`, not `  206ms`) so it sits tight against the bar.
