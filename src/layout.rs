//! Pure sizing math: how wide each column of a monitor row is, how many
//! columns fit, and how many monitors can be shown.
//!
//! Nothing here touches a terminal, so every breakpoint is unit-testable.

/// Column widths of one monitor row. A width of 0 means "omit this column".
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RowShape {
    pub name_w: u16,
    pub uptime_w: u16,
    pub bar_w: u16,
    pub ping_w: u16,
}

/// What to draw this frame.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Plan {
    pub shape: RowShape,
    pub columns: u16,
    /// Monitors per column, not screen rows - `row_gap` is extra.
    pub rows_per_col: u16,
    pub shown: usize,
    pub hidden: usize,
    /// Blank screen rows between monitor rows. Full-height blocks in adjacent
    /// rows otherwise touch and read as one solid mass.
    pub row_gap: u16,
    /// Whether the status line is set off by a blank row above and below.
    pub header_gap: bool,
}

/// Widest bar worth drawing: the API only ever returns ~50 beats.
const MAX_BAR: u16 = 50;
/// Below this a bar is more noise than signal, so the column is dropped.
const MIN_BAR: u16 = 10;
/// Beyond this, columns are too narrow to be worth reading.
const MAX_COLUMNS: u16 = 6;
/// `"1128ms"` - compact, no alignment padding, so it sits tight against the
/// bar and stays affordable even in a narrow column.
const PING_W: u16 = 6;
/// One blank row above the status line and one below, so it sits balanced in
/// its own band rather than crowded against the top border.
const HEADER_GAP_ROWS: u16 = 2;

/// Column widths for a monitor row given the width of its column.
///
/// Name, uptime and ping are the load-bearing columns and are kept as long as
/// they physically fit. The bar is the elastic one: it takes whatever is left,
/// shrinking as the column narrows, and is dropped once it is too short to
/// read - a six-beat stub says less than the ping figure it would displace.
pub fn row_shape(col_w: u16, has_marker: bool) -> RowShape {
    // left pad + group marker + "* " + right pad
    let fixed = 1 + u16::from(has_marker) + 2 + 1;
    let rem = col_w.saturating_sub(fixed);

    if col_w >= 40 {
        let (name_w, uptime_w) = if col_w >= 90 {
            (20, 7)
        } else if col_w >= 60 {
            (16, 6)
        } else {
            (14, 4)
        };
        // three separators: before uptime, before the bar, before ping
        let bar_w = rem.saturating_sub(name_w + uptime_w + PING_W + 3);
        return RowShape {
            name_w,
            uptime_w,
            bar_w: keep_bar(bar_w),
            ping_w: PING_W,
        };
    }

    // Too narrow for a bar at all. Keep ping while the name stays legible,
    // then give it up so the name is not reduced to an ellipsis.
    let uptime_w = 4;
    let shortest_name = 6;
    if rem >= shortest_name + uptime_w + PING_W + 2 {
        RowShape {
            name_w: rem.saturating_sub(uptime_w + PING_W + 2),
            uptime_w,
            bar_w: 0,
            ping_w: PING_W,
        }
    } else {
        RowShape {
            name_w: rem.saturating_sub(uptime_w + 1).max(shortest_name),
            uptime_w,
            bar_w: 0,
            ping_w: 0,
        }
    }
}

fn keep_bar(bar_w: u16) -> u16 {
    if bar_w >= MIN_BAR {
        bar_w.min(MAX_BAR)
    } else {
        0
    }
}

/// Lays out `n` monitors inside a frame of `width` x `height`.
///
/// `banner_rows` is the incident/maintenance lines, which the caller counts.
/// The legend lives in the bottom border, so it costs no rows.
pub fn plan(
    width: u16,
    height: u16,
    n: usize,
    has_marker: bool,
    max: Option<usize>,
    banner_rows: u16,
) -> Plan {
    let inner_w = width.saturating_sub(2);
    // two borders, the overall-status line, and any banners
    let avail = height.saturating_sub(3 + banner_rows);

    if avail == 0 || inner_w == 0 || n == 0 {
        return Plan {
            shape: row_shape(inner_w, has_marker),
            columns: 1,
            rows_per_col: 0,
            shown: 0,
            hidden: n,
            row_gap: 0,
            header_gap: false,
        };
    }

    // `--max` is a deliberate cap, so lay out for what will actually be drawn
    // rather than spreading a handful of rows across needlessly thin columns.
    let target = max.map_or(n, |m| m.min(n));

    // Breathing room is worth having but never worth hiding a monitor for, so
    // try the roomiest arrangement first and give the spacing back - the row
    // gap before the header gap, since the gap costs a row per monitor and the
    // header costs one row in total - until everything fits. If nothing fits,
    // the last (tightest) arrangement is the right one anyway.
    let ladder = [(true, 1u16), (true, 0), (false, 0)];
    let mut arrangement = ladder[ladder.len() - 1];
    for candidate in ladder {
        arrangement = candidate;
        if fits(inner_w, avail, target, has_marker, candidate) {
            break;
        }
    }
    let (header_gap, row_gap) = arrangement;

    let rows = avail.saturating_sub(header_rows(header_gap));
    let columns = columns_for(inner_w, rows, target, has_marker, row_gap);
    let capacity = (columns as usize) * per_column(rows, row_gap);
    let shown = target.min(capacity);

    Plan {
        shape: row_shape(inner_w / columns, has_marker),
        columns,
        rows_per_col: shown.div_ceil(columns as usize) as u16,
        shown,
        hidden: n - shown,
        row_gap,
        header_gap,
    }
}

/// How many monitors a column of `rows` screen rows holds, given the gap
/// between them. The last monitor needs no trailing gap.
fn per_column(rows: u16, gap: u16) -> usize {
    if rows == 0 {
        return 0;
    }
    ((rows + gap) / (1 + gap)) as usize
}

/// Fewest columns that hold `target` monitors, subject to every column still
/// fitting a legible bar - a wall of name-only slivers is worse than hiding a
/// few monitors.
fn columns_for(inner_w: u16, rows: u16, target: usize, has_marker: bool, gap: u16) -> u16 {
    let mut columns: u16 = 1;
    while columns as usize * per_column(rows, gap) < target
        && columns < MAX_COLUMNS
        && row_shape(inner_w / (columns + 1), has_marker).bar_w >= MIN_BAR
    {
        columns += 1;
    }
    columns
}

fn fits(
    inner_w: u16,
    avail: u16,
    target: usize,
    has_marker: bool,
    (header_gap, row_gap): (bool, u16),
) -> bool {
    let rows = avail.saturating_sub(header_rows(header_gap));
    let columns = columns_for(inner_w, rows, target, has_marker, row_gap);
    columns as usize * per_column(rows, row_gap) >= target
}

fn header_rows(header_gap: bool) -> u16 {
    if header_gap { HEADER_GAP_ROWS } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_screen_gets_the_whole_row() {
        let s = row_shape(120, true);
        assert_eq!(
            s,
            RowShape {
                name_w: 20,
                uptime_w: 7,
                bar_w: 50,
                ping_w: PING_W
            }
        );
    }

    #[test]
    fn the_bar_shrinks_so_that_ping_survives() {
        // Wide: everything, bar capped at 50.
        let s = row_shape(200, true);
        assert_eq!((s.bar_w, s.ping_w), (MAX_BAR, PING_W));
        // Medium: shorter name, ping kept, bar takes the remainder.
        let s = row_shape(75, true);
        assert_eq!((s.name_w, s.uptime_w, s.ping_w), (16, 6, PING_W));
        assert!(s.bar_w >= MIN_BAR);
        // Two columns of a 120-wide frame: ping must still be there.
        let s = row_shape(59, true);
        assert_eq!(s.ping_w, PING_W, "a 59-wide column keeps ping");
        assert!(s.bar_w >= MIN_BAR, "and still shows a readable bar");
        // Narrow: the bar goes, ping stays.
        let s = row_shape(38, true);
        assert_eq!((s.bar_w, s.ping_w), (0, PING_W));
        // Only when the name would be squeezed to nothing does ping go too.
        let s = row_shape(20, true);
        assert_eq!((s.bar_w, s.ping_w), (0, 0));
        assert!(s.name_w >= 6);
    }

    #[test]
    fn a_bar_is_never_left_as_a_useless_stub() {
        for w in 0..=200u16 {
            for marker in [true, false] {
                let b = row_shape(w, marker).bar_w;
                assert!(b == 0 || b >= MIN_BAR, "width {w} gave a {b}-wide bar");
                assert!(b <= MAX_BAR);
                // Once a bar is affordable, ping must already be affordable.
                if b > 0 {
                    assert_eq!(
                        row_shape(w, marker).ping_w,
                        PING_W,
                        "width {w} showed a bar but no ping"
                    );
                }
            }
        }
    }

    #[test]
    fn a_row_never_claims_more_than_its_column() {
        // Widths where the ladder promises a fit; below ~20 the terminal is
        // simply too narrow and ratatui clips.
        for w in 20..=200u16 {
            for marker in [true, false] {
                let s = row_shape(w, marker);
                let seps =
                    u16::from(s.uptime_w > 0) + u16::from(s.bar_w > 0) + u16::from(s.ping_w > 0);
                let used =
                    1 + u16::from(marker) + 2 + s.name_w + s.uptime_w + s.bar_w + s.ping_w + seps;
                assert!(used <= w, "width {w} marker {marker}: row needs {used}");
            }
        }
    }

    #[test]
    fn a_roomy_frame_shows_everything_with_breathing_room() {
        let p = plan(120, 44, 19, true, None, 0);
        assert_eq!(p.columns, 1);
        assert_eq!(p.shown, 19);
        assert_eq!(p.hidden, 0);
        assert_eq!(p.shape.bar_w, MAX_BAR);
        assert_eq!(p.row_gap, 1, "there is room to space the rows out");
        assert!(p.header_gap);
    }

    #[test]
    fn spacing_may_cost_a_column_but_never_a_monitor() {
        // 19 monitors + 18 gaps needs 37 rows; only 33 are free, so the rows
        // flow into a second column rather than losing the spacing.
        let p = plan(120, 36, 19, true, None, 0);
        assert_eq!(p.shown, 19, "no monitor is hidden");
        assert_eq!(p.hidden, 0);
        assert_eq!(p.row_gap, 1);
        assert_eq!(p.columns, 2);
    }

    #[test]
    fn spacing_is_given_up_before_a_monitor_is_hidden() {
        // Tall and narrow: a second column will not fit, so the gap goes
        // instead - showing every monitor matters more than looking pretty.
        let p = plan(50, 24, 19, true, None, 0);
        assert_eq!(p.columns, 1);
        assert_eq!(p.row_gap, 0);
        assert_eq!(p.shown, 19, "compact fits all 19");
        assert_eq!(p.hidden, 0);
    }

    #[test]
    fn the_row_gap_is_surrendered_before_the_header_gap() {
        // Room for 19 monitors plus the header band, but nowhere near enough
        // for 18 row gaps.
        let p = plan(50, 24, 19, true, None, 0);
        assert_eq!(p.row_gap, 0);
        assert!(p.header_gap, "the header band costs two rows in total");
        assert_eq!(p.shown, 19);
    }

    #[test]
    fn the_header_gap_is_surrendered_before_a_monitor_is_hidden() {
        // Two rows short of affording the header band, but every monitor
        // still fits without it.
        let p = plan(50, 23, 19, true, None, 0);
        assert!(!p.header_gap);
        assert_eq!(p.row_gap, 0);
        assert_eq!(p.shown, 19, "nothing is hidden for the sake of a blank row");
        assert_eq!(p.hidden, 0);
    }

    #[test]
    fn a_tiled_pane_truncates_and_reports_the_remainder() {
        let p = plan(50, 15, 19, true, None, 0);
        assert_eq!(p.columns, 1, "50 cols cannot hold two legible bars");
        assert_eq!(p.shown, 12);
        assert_eq!(p.hidden, 7);
    }

    #[test]
    fn a_wide_short_frame_flows_into_columns() {
        let p = plan(200, 12, 40, true, None, 0);
        assert!(p.columns >= 2, "got {} columns", p.columns);
        assert_eq!(p.shown + p.hidden, 40);
        assert!(p.rows_per_col as usize * p.columns as usize >= p.shown);
    }

    #[test]
    fn max_caps_what_is_shown() {
        let p = plan(120, 36, 19, true, Some(5), 0);
        assert_eq!(p.shown, 5);
        assert_eq!(p.hidden, 14);
    }

    #[test]
    fn max_does_not_spread_a_few_rows_over_many_columns() {
        // 4 monitors on a wide short frame belong in one column, not four.
        let p = plan(200, 12, 19, true, Some(4), 0);
        assert_eq!(p.shown, 4);
        assert_eq!(p.columns, 1);
        assert_eq!(p.hidden, 15);
    }

    #[test]
    fn banners_take_rows_away_from_monitors() {
        let a = plan(120, 10, 40, true, None, 0);
        let b = plan(120, 10, 40, true, None, 2);
        assert_eq!(a.shown - b.shown, 2 * a.columns as usize);
    }

    #[test]
    fn degenerate_frames_do_not_panic_or_overflow() {
        for w in 0..12u16 {
            for h in 0..8u16 {
                let p = plan(w, h, 19, true, None, 0);
                assert_eq!(p.shown + p.hidden, 19);
                assert!(p.columns >= 1);
            }
        }
        assert_eq!(plan(80, 3, 5, true, None, 0).shown, 0);
        assert_eq!(plan(80, 24, 0, true, None, 0).shown, 0);
    }
}
