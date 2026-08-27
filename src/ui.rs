//! Drawing. All widths are derived from the frame on every draw, so a resize
//! is just the next frame - nothing is cached and nothing assumes a size.
//!
//! Widths come from `layout::plan`; this module only turns them into spans.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::args::Args;
use crate::glyphs::Glyphs;
use crate::layout::{self, Plan, RowShape};
use crate::model::{Monitor, Overall, State, Status};

/// One space of breathing room inside the left border. Rendered as a span
/// rather than by offsetting the draw area, so a long line can never spill
/// past the right border.
const LEFT_PAD: &str = " ";

pub fn draw(f: &mut Frame, s: &State, args: &Args) {
    let g = Glyphs::new(args.ascii);
    let area = f.area();

    let has_marker = s.groups.len() > 1;
    let banner_rows = u16::from(s.incident.is_some()) + u16::from(!s.maintenance.is_empty());
    let plan = layout::plan(
        area.width,
        area.height,
        s.monitors.len(),
        has_marker,
        args.max,
        banner_rows,
    );

    let block = outer_block(s, &plan, &g);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut header = header_lines(s, &g);
    if plan.header_gap {
        // Balanced: the status line gets a blank row on each side.
        header.insert(0, Line::default());
        header.push(Line::default());
    }
    let [head, grid] =
        Layout::vertical([Constraint::Length(header.len() as u16), Constraint::Fill(1)])
            .areas(inner);
    f.render_widget(Paragraph::new(header), head);

    if plan.shown == 0 || plan.rows_per_col == 0 {
        return;
    }
    render_grid(f, s, &plan, has_marker, &g, grid);
}

/// Monitors fill each column top to bottom before starting the next, so
/// reading order matches the priority order from `display_order`.
fn render_grid(f: &mut Frame, s: &State, plan: &Plan, has_marker: bool, g: &Glyphs, grid: Rect) {
    let order = s.display_order();
    let areas = Layout::horizontal(vec![Constraint::Fill(1); plan.columns as usize]).split(grid);

    for (c, area) in areas.iter().enumerate() {
        let start = c * plan.rows_per_col as usize;
        let end = (start + plan.rows_per_col as usize).min(plan.shown);
        if start >= end {
            break;
        }
        let mut lines: Vec<Line> = Vec::new();
        for (k, &i) in order[start..end].iter().enumerate() {
            // A gap before every row but the first: full-height blocks in
            // adjacent rows would otherwise merge into one solid mass.
            if k > 0 {
                for _ in 0..plan.row_gap {
                    lines.push(Line::default());
                }
            }
            lines.push(monitor_line(&s.monitors[i], plan.shape, has_marker, g));
        }
        f.render_widget(Paragraph::new(lines), *area);
    }
}

fn header_lines<'a>(s: &State, g: &Glyphs) -> Vec<Line<'a>> {
    let mut lines = vec![overall_line(s, g)];
    lines.extend(banner_lines(s, g));
    lines
}

fn outer_block<'a>(s: &'a State, plan: &Plan, g: &Glyphs) -> Block<'a> {
    let updated = match s.last_ok {
        // Elapsed time, not a clock time: no timezone handling anywhere.
        Some(t) => format!(" updated {}s ago ", t.elapsed().as_secs()),
        None => " no data ".to_string(),
    };

    let mut block = Block::bordered()
        .border_set(g.border())
        .title(Span::styled(
            format!(" {} ", s.title),
            Style::default().add_modifier(Modifier::BOLD),
        ))
        .title_top(
            Line::from(Span::styled(updated, Style::default().fg(Color::Gray))).right_aligned(),
        )
        .title_bottom(Line::from(right_status(plan)).right_aligned());

    if s.groups.len() > 1 {
        let mut spans = vec![Span::raw(" ")];
        for (i, name) in s.groups.iter().enumerate() {
            let (shape, color) = g.marker(i);
            spans.push(Span::styled(shape.to_string(), Style::default().fg(color)));
            spans.push(Span::styled(
                format!(" {name}  "),
                Style::default().fg(Color::Gray),
            ));
        }
        block = block.title_bottom(Line::from(spans));
    }
    block
}

/// Whatever could not be shown is announced rather than silently dropped.
fn right_status<'a>(plan: &Plan) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    if plan.hidden > 0 {
        spans.push(Span::styled(
            format!(" +{} more ", plan.hidden),
            Style::default().fg(Color::Yellow),
        ));
    }
    spans.push(Span::styled(" q quit ", Style::default().fg(Color::Gray)));
    spans
}

fn overall_line<'a>(s: &State, g: &Glyphs) -> Line<'a> {
    // A failed fetch outranks whatever the stale data would otherwise say,
    // and the reason is shown rather than a generic "cannot reach": knowing it
    // was a bad slug rather than a dead network is the whole point.
    if let Some(e) = &s.error {
        return status_line(g.bad(), format!("{e} (retrying)"), Color::Red);
    }
    let (glyph, text, color) = match s.overall() {
        Overall::Operational => (g.ok(), "All Systems Operational", Color::Green),
        // Red, not yellow: on a wall display "something is down" has to read
        // as trouble at a glance.
        Overall::PartiallyDegraded => (g.bad(), "Partially Degraded Service", Color::Red),
        Overall::Degraded => (g.bad(), "Degraded Service", Color::Red),
        Overall::Maintenance => (g.maint(), "Under Maintenance", Color::Blue),
        Overall::Pending => (g.pending(), "Some Checks Pending", Color::Yellow),
        Overall::Unknown => (g.pending(), "Connecting", Color::Gray),
    };
    status_line(glyph, text.to_string(), color)
}

fn status_line<'a>(glyph: &str, text: String, color: Color) -> Line<'a> {
    Line::from(vec![
        Span::raw(LEFT_PAD),
        Span::styled(
            format!("{glyph} {text}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

fn banner_lines<'a>(s: &State, g: &Glyphs) -> Vec<Line<'a>> {
    let mut out = Vec::new();
    if let Some((title, color)) = &s.incident {
        out.push(Line::from(vec![
            Span::raw(LEFT_PAD),
            Span::styled(
                format!("{} Incident: {title}", g.incident()),
                Style::default().fg(*color),
            ),
        ]));
    }
    if !s.maintenance.is_empty() {
        out.push(Line::from(vec![
            Span::raw(LEFT_PAD),
            Span::styled(
                format!("{} Maintenance: {}", g.maint(), s.maintenance.join("; ")),
                Style::default().fg(Color::Blue),
            ),
        ]));
    }
    out
}

fn monitor_line<'a>(m: &Monitor, shape: RowShape, has_marker: bool, g: &Glyphs) -> Line<'a> {
    let mut spans = vec![Span::raw(LEFT_PAD)];

    if has_marker {
        let (marker, color) = g.marker(m.group);
        spans.push(Span::styled(marker.to_string(), Style::default().fg(color)));
    }
    spans.push(Span::styled(
        format!("{} ", g.dot()),
        Style::default().fg(m.current.color()),
    ));
    spans.push(Span::raw(pad(&m.name, shape.name_w as usize, g)));

    if shape.uptime_w > 0 {
        spans.push(Span::styled(
            format!(
                " {:>w$}",
                uptime_text(m.uptime24, shape.uptime_w),
                w = shape.uptime_w as usize
            ),
            Style::default().fg(Color::Gray),
        ));
    }
    if shape.bar_w > 0 {
        spans.push(Span::raw(" "));
        spans.extend(bar_spans(m, shape.bar_w as usize, g));
    }
    if shape.ping_w > 0 {
        spans.push(Span::styled(
            format!(" {}", ping_text(m.latest_ping, shape.ping_w)),
            Style::default().fg(Color::Gray),
        ));
    }
    Line::from(spans)
}

/// One span per beat, newest at the right.
///
/// Each cell does double duty: colour is status, height is latency. A down
/// beat is forced to full height as well as red, so it can never be mistaken
/// for a merely slow one.
fn bar_spans<'a>(m: &Monitor, width: usize, g: &Glyphs) -> Vec<Span<'a>> {
    let mut spans = Vec::with_capacity(width + 1);

    // Fewer beats than the bar is wide: pad the left so the newest beat still
    // lands on the right edge.
    let missing = width.saturating_sub(m.beats.len());
    if missing > 0 {
        spans.push(Span::styled(
            g.beat(0).to_string().repeat(missing),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let start = m.beats.len().saturating_sub(width);
    let shown = &m.beats[start..];
    // Scale over the beats actually on screen, so a narrow pane rescales to
    // what it is showing rather than to history it cropped away.
    let (lo, hi) = crate::model::ping_scale(shown);

    for b in shown {
        let status = Status::from_code(b.status);
        let level = match status {
            Status::Down => 7,
            _ => crate::model::ping_level(b.ping, lo, hi),
        };
        spans.push(Span::styled(
            g.beat(level).to_string(),
            Style::default().fg(status.color()),
        ));
    }
    spans
}

/// Uptime at whatever precision the column affords.
///
/// Never rounds up to a perfect score: at zero decimals `{:.0}` would turn
/// 99.93% into "100%", which on a status page is a claim of no downtime that
/// is simply untrue. Values below 100 are floored instead, so the number is
/// pessimistic rather than flattering.
fn uptime_text(uptime: Option<f64>, width: u16) -> String {
    let Some(u) = uptime else {
        return "--".to_string();
    };
    let decimals: usize = match width {
        0..=4 => 0,
        5..=6 => 1,
        _ => 2,
    };
    let pct = u * 100.0;
    let shown = if u < 1.0 {
        let factor = 10f64.powi(decimals as i32);
        ((pct * factor).floor() / factor).min(100.0 - 1.0 / factor)
    } else {
        pct
    };
    format!("{shown:.decimals$}%")
}

/// Compact, so it can sit tight against the bar: `"206ms"`, not `"  206ms"`.
/// Right-aligned in its column so the digits still line up down the screen.
fn ping_text(ping: Option<f64>, width: u16) -> String {
    let w = width as usize;
    match ping {
        Some(p) => format!("{:>w$}", format!("{}ms", p.round() as i64)),
        None => " ".repeat(w),
    }
}

/// Truncate by character count and pad to an exact width. Names are effectively
/// ASCII in practice, so this does not pull in a unicode-width dependency.
fn pad(s: &str, width: usize, g: &Glyphs) -> String {
    let n = s.chars().count();
    if n <= width {
        format!("{s}{}", " ".repeat(width - n))
    } else {
        let keep: String = s.chars().take(width.saturating_sub(1)).collect();
        format!("{keep}{}", g.ellipsis())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_precision_follows_the_column_width() {
        assert_eq!(uptime_text(Some(1.0), 7), "100.00%");
        assert_eq!(uptime_text(Some(1.0), 6), "100.0%");
        assert_eq!(uptime_text(Some(1.0), 4), "100%");
        assert_eq!(uptime_text(Some(0.98654), 7), "98.65%");
        assert_eq!(uptime_text(Some(0.5), 4), "50%");
        assert_eq!(uptime_text(None, 7), "--");
    }

    #[test]
    fn uptime_never_claims_a_perfect_score_it_does_not_have() {
        assert_eq!(uptime_text(Some(0.9993), 4), "99%");
        assert_eq!(uptime_text(Some(0.9993), 6), "99.9%");
        assert_eq!(uptime_text(Some(0.999999), 7), "99.99%");
        // Only a genuine 100% prints as 100%.
        assert_eq!(uptime_text(Some(1.0), 4), "100%");
        assert_eq!(uptime_text(Some(1.0), 6), "100.0%");
        assert_eq!(uptime_text(Some(1.0), 7), "100.00%");
    }

    #[test]
    fn uptime_never_exceeds_its_column() {
        for w in [4u16, 6, 7] {
            let s = uptime_text(Some(1.0), w);
            assert!(s.len() <= w as usize, "{s:?} overflows width {w}");
        }
    }

    #[test]
    fn ping_fills_its_column_exactly() {
        for p in [Some(0.0), Some(72.4), Some(999.0), Some(1234.0), None] {
            assert_eq!(ping_text(p, 6).chars().count(), 6, "{p:?}");
        }
        assert_eq!(ping_text(Some(206.0), 6), " 206ms");
        assert_eq!(ping_text(Some(1128.0), 6), "1128ms");
    }

    #[test]
    fn pad_truncates_and_fills_to_an_exact_width() {
        let g = Glyphs::new(true);
        assert_eq!(pad("abc", 5, &g), "abc  ");
        assert_eq!(pad("abcdef", 4, &g), "abc~");
        assert_eq!(pad("abcd", 4, &g), "abcd");
        for name in ["", "a", "a much longer monitor name than fits"] {
            assert_eq!(pad(name, 8, &g).chars().count(), 8);
        }
    }
}
