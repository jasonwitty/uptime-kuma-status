//! Render a public [Uptime Kuma](https://github.com/louislam/uptime-kuma)
//! status page in the terminal.
//!
//! ```text
//! uptime-kuma-status https://status.example.com/status/myslug
//! ```
//!
//! # Scope
//!
//! Display only, and only for *public* status pages. There is no
//! authentication, no socket.io, and no way to change anything on the server.
//! The whole program reads the two unauthenticated JSON endpoints that a
//! public status page already serves.
//!
//! # Shape of the program
//!
//! One HTTP fetch per interval on a worker thread, one draw per frame on the
//! main thread, and an `mpsc` channel between them. No async runtime: a single
//! request a minute does not justify one.
//!
//! | Module | Responsibility |
//! | --- | --- |
//! | [`args`] | command line, help text |
//! | [`api`] | the two endpoints and their JSON |
//! | [`model`] | flattened monitors, status, ordering, ping scaling |
//! | [`layout`] | pure sizing maths - which columns fit, and how many |
//! | [`ui`] | turning those widths into ratatui spans |
//! | [`glyphs`] | every character drawn, plus the ASCII fallback |
//!
//! # Design notes
//!
//! *Nothing scrolls.* Every width is derived from the frame on each draw, and
//! the layout gives things up in a fixed order as space runs out. See
//! [`layout::row_shape`] for that ladder.
//!
//! *Only named terminal colours are used*, never `Color::Rgb`, so the user's
//! terminal theme is the program's theme.

mod api;
mod args;
mod glyphs;
mod layout;
mod model;
mod ui;

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use api::StatusPage;
use model::{Msg, State};

/// How often the event loop wakes to repaint and check the keyboard. Also the
/// granularity at which the fetch thread notices a refresh request.
const TICK: Duration = Duration::from_millis(250);

fn main() -> ExitCode {
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(Some(a)) => a,
        Ok(None) => {
            print!("{}", args::HELP);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}\n");
            eprint!("{}", args::HELP);
            return ExitCode::from(2);
        }
    };

    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: args::Args) -> Result<(), String> {
    // Only what can be judged offline is judged here, so that a malformed URL
    // is a usage error on stderr rather than a banner inside a TUI the user
    // then has to quit. Resolving the slug may need a request of its own; that
    // waits for the fetch thread.
    let title = api::precheck(&args.url)?;

    let (tx, rx) = mpsc::channel();
    let refresh = Arc::new(AtomicBool::new(false));
    spawn_fetcher(args.url.clone(), args.interval, tx, Arc::clone(&refresh));

    let mut state = State::new(title);

    // Installs a panic hook that restores the terminal, so a crash cannot
    // leave the user in raw mode with no echo.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut state, &args, &rx, &refresh);
    ratatui::restore();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    state: &mut State,
    args: &args::Args,
    rx: &Receiver<Msg>,
    refresh: &AtomicBool,
) -> Result<(), String> {
    loop {
        while let Ok(msg) = rx.try_recv() {
            state.apply(msg);
        }

        terminal
            .draw(|f| ui::draw(f, state, args))
            .map_err(|e| e.to_string())?;

        if !event::poll(TICK).map_err(|e| e.to_string())? {
            continue;
        }
        // Key repeats and releases arrive on some platforms; only presses act.
        if let Event::Key(k) = event::read().map_err(|e| e.to_string())?
            && k.kind == KeyEventKind::Press
        {
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Char('r') => refresh.store(true, Ordering::Relaxed),
                _ => {}
            }
        }
    }
}

/// Fetches both endpoints forever, handing every outcome to the UI as a [`Msg`].
///
/// The config endpoint is re-fetched each cycle rather than once at startup, so
/// a monitor added to the status page shows up without a restart. It is a few
/// kilobytes once a minute.
///
/// The URL is resolved here rather than by the caller because
/// [`StatusPage::resolve`] can cost an HTTP request, and nothing that can
/// block for ten seconds belongs in front of the terminal: the UI draws and
/// answers the keyboard while this runs. A page that cannot be resolved yet is
/// reported like any other fetch failure and tried again next cycle, so a host
/// that is down at startup needs no restart either.
fn spawn_fetcher(url: String, interval: u64, tx: Sender<Msg>, refresh: Arc<AtomicBool>) {
    thread::spawn(move || {
        let mut page: Option<StatusPage> = None;
        loop {
            let msg = match page {
                Some(ref p) => fetch(p),
                None => match StatusPage::resolve(&url) {
                    Ok(p) => fetch(page.insert(p)),
                    Err(e) => Msg::Error(e),
                },
            };
            // The receiver is gone: the UI has quit, so should this thread.
            if tx.send(msg).is_err() {
                return;
            }
            sleep_or_refresh(interval, &refresh);
        }
    });
}

/// One cycle of both endpoints, folded into a single message for the UI.
fn fetch(page: &StatusPage) -> Msg {
    match (api::fetch_config(page), api::fetch_heartbeat(page)) {
        (Ok(cfg), Ok(hb)) => Msg::Data(Box::new(cfg), Box::new(hb)),
        (Err(e), _) | (_, Err(e)) => Msg::Error(e),
    }
}

/// Sleeps in short slices so that pressing `r` is felt immediately rather than
/// up to a whole interval later.
fn sleep_or_refresh(interval: u64, refresh: &AtomicBool) {
    let slices = (interval * 1000) / TICK.as_millis() as u64;
    for _ in 0..slices {
        if refresh.swap(false, Ordering::Relaxed) {
            return;
        }
        thread::sleep(TICK);
    }
}
