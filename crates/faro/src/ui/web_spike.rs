//! M13 Phase 0 spike (throwaway): export real faro frames for the web renderer
//! prototypes (WEBSITE.md §6.3, D-d).
//!
//! Builds the demo as a visitor would see it: the mock's Scranton cast and a
//! deterministic event stream (seed 42, no outages), stored in an in-memory
//! store, loaded into a `State` through the real reducer, rendered by
//! `ui::render` (including `emoji::finish`). Writes each frame's cells as JSON.
//!
//! `FARO_SPIKE_OUT=<dir> cargo test -p faro web_spike -- --ignored`

use std::fmt::Write as _;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use faro_core::backend::{Backend, BackendEvent};
use faro_core::config::BubbleStyle;
use faro_core::domain::{ConnectionState, ConversationId};
use faro_core::mock::{DETERMINISTIC_EPOCH, MockBackend, MockConfig};
use faro_core::store::Store;
use jiff::tz::TimeZone;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

use super::{View, render};
use crate::app::event::AppEvent;
use crate::app::images::Images;
use crate::app::state::{Geometry, MessageLayout, State};
use crate::app::update::update;
use crate::theme::Theme;
use crate::theme::UserThemes;

const WIDTH: u16 = 120;
const HEIGHT: u16 = 34;
/// Events stored before the visitor "opens" faro, then events played as a flood.
const BACKLOG_EVENTS: usize = 220;
const FLOOD_EVENTS: usize = 60;
const THEMES: [&str; 7] = [
    "faro-dark",
    "faro-light",
    "faro-dusk",
    "catppuccin",
    "nord",
    "gruvbox",
    "tokyonight",
];
/// Pasted into the input and sent: flags and ZWJ sequences (§6.3).
const EMOJI_MESSAGE: &str = "Flags for the party: 🇫🇷 🇮🇹 🇯🇵 🇲🇽 · and 🏳️‍🌈 🏴‍☠️ 👩‍👩‍👧 💁🏽‍♀️ 🧗‍♀️ for the banner";

struct Demo {
    me: faro_core::domain::ServiceId,
    contacts: Vec<faro_core::domain::Contact>,
    groups: Vec<faro_core::domain::Group>,
    backlog: Vec<BackendEvent>,
    flood: Vec<BackendEvent>,
}

/// The mock on a paused clock: the same events on every run.
async fn record() -> Demo {
    let (backend, mut events) = MockBackend::new(&MockConfig {
        disconnects: false,
        errors: false,
        ..MockConfig::deterministic(42)
    });
    let me = backend.whoami().await.unwrap().service_id;
    let contacts = backend.contacts().await.unwrap();
    let groups = backend.groups().await.unwrap();
    let mut all = Vec::new();
    while all.len() < BACKLOG_EVENTS + FLOOD_EVENTS {
        all.push(events.recv().await.unwrap());
    }
    let flood = all.split_off(BACKLOG_EVENTS);
    Demo {
        me,
        contacts,
        groups,
        backlog: all,
        flood,
    }
}

fn key(state: &mut State, code: KeyCode, modifiers: KeyModifiers) {
    update(
        state,
        AppEvent::Terminal(Event::Key(KeyEvent::new(code, modifiers))),
    );
}

/// What faro shows after a restart with that history: the list from the
/// store, Scranton Branch open, bubbles with rounded corners.
async fn loaded(demo: &Demo) -> (State, ConversationId) {
    let store = Store::open_in_memory().await.unwrap();
    store.upsert_contacts(demo.contacts.clone()).await.unwrap();
    store.upsert_groups(demo.groups.clone()).await.unwrap();
    for event in &demo.backlog {
        store.apply(event.clone()).await.unwrap();
    }
    let scranton = demo
        .groups
        .iter()
        .find(|g| g.title.starts_with("Scranton Branch"))
        .map(|g| ConversationId::Group(g.id))
        .unwrap();
    let mut state = State::new(demo.me, demo.contacts.clone(), demo.groups.clone());
    state.layout = MessageLayout::Bubbles;
    state.config.messages.layout = MessageLayout::Bubbles;
    state.config.messages.bubble_style = BubbleStyle::Rounded;
    update(
        &mut state,
        AppEvent::Loaded {
            conversations: store.conversations().await.unwrap(),
            last_open: Some(scranton),
            recent_reactions: Vec::new(),
        },
    );
    key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    update(
        &mut state,
        AppEvent::MessagesLoaded {
            conversation: scranton,
            messages: store.messages(scranton, None, 200).await.unwrap(),
        },
    );
    update(
        &mut state,
        AppEvent::Backend(BackendEvent::Connection(ConnectionState::Connected)),
    );
    // The visitor sends flags and ZWJ emoji (a local echo, as in the app).
    update(
        &mut state,
        AppEvent::Terminal(Event::Paste(EMOJI_MESSAGE.to_owned())),
    );
    key(&mut state, KeyCode::Enter, KeyModifiers::NONE);
    (state, scranton)
}

fn draw(state: &State, theme: &Theme, logo_time: Option<f64>, minutes: u64) -> Buffer {
    let tz = TimeZone::UTC;
    let now_ms = DETERMINISTIC_EPOCH.as_millis() + minutes * 60_000;
    let now = jiff::Timestamp::from_millisecond(i64::try_from(now_ms).unwrap()).unwrap();
    let mut images = Images::halfblocks();
    images.prepare_frame(state);
    let view = View {
        theme,
        tz: &tz,
        now,
        logo_time,
        images: &images,
    };
    let mut geometry = Geometry::default();
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal
        .draw(|frame| geometry = render(frame, state, &view))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn color(c: Color) -> String {
    if let Color::Rgb(r, g, b) = c {
        format!("\"#{r:02x}{g:02x}{b:02x}\"")
    } else if c == Color::Reset {
        "null".to_owned()
    } else {
        format!("\"{c}\"")
    }
}

fn json_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `{"w":..,"h":..,"theme":{..},"cells":[[symbol, width, fg, bg, flags], ..]}`
/// row by row. `width` is the cells faro gave the symbol (its `ForcedWidth`,
/// else Unicode width); `flags`: 1 bold, 2 dim, 4 italic, 8 underline,
/// 16 reversed, 32 crossed out, 64 skip (an image cell).
fn export(buffer: &Buffer, theme: &Theme, name: &str) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "{{\"name\":\"{name}\",\"w\":{},\"h\":{},\"theme\":{{\"bg\":{},\"fg\":{}}},\"cells\":[",
        buffer.area.width,
        buffer.area.height,
        color(theme.background),
        color(theme.text)
    );
    for (i, cell) in buffer.content.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let width = match cell.diff_option {
            CellDiffOption::ForcedWidth(w) => usize::from(w.get()),
            CellDiffOption::None | CellDiffOption::Skip | CellDiffOption::AlwaysUpdate => {
                cell.symbol().width()
            }
        };
        let m = cell.modifier;
        let flags = u8::from(m.contains(Modifier::BOLD))
            | u8::from(m.contains(Modifier::DIM)) << 1
            | u8::from(m.contains(Modifier::ITALIC)) << 2
            | u8::from(m.contains(Modifier::UNDERLINED)) << 3
            | u8::from(m.contains(Modifier::REVERSED)) << 4
            | u8::from(m.contains(Modifier::CROSSED_OUT)) << 5
            | u8::from(cell.diff_option == CellDiffOption::Skip) << 6;
        out.push('[');
        json_string(cell.symbol(), &mut out);
        let _ = write!(
            out,
            ",{width},{},{},{flags}]",
            color(cell.fg),
            color(cell.bg)
        );
    }
    out.push_str("]}");
    out
}

#[test]
#[ignore = "M13 spike: writes frames for the web renderer prototypes"]
#[expect(
    clippy::disallowed_methods,
    reason = "a throwaway exporter: blocks on runtimes and writes files"
)]
fn web_spike_frames() {
    let Ok(out) = std::env::var("FARO_SPIKE_OUT") else {
        return;
    };
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();

    // Paused clock for the mock; a real one for SQLite (AGENTS.md gotcha).
    let paused = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let demo = paused.block_on(record());
    drop(paused);
    let real = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut state, _) = real.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), loaded(&demo))
            .await
            .unwrap()
    });

    let user = UserThemes::default();
    let mut index = Vec::new();
    let mut write = |name: String, buffer: &Buffer, theme: &Theme| {
        std::fs::write(
            out.join(format!("{name}.json")),
            export(buffer, theme, &name),
        )
        .unwrap();
        index.push(name);
    };

    for name in THEMES {
        let Some(theme) = Theme::find(name, &user) else {
            continue;
        };
        state.config.theme = name.to_owned();
        write(format!("conv-{name}"), &draw(&state, theme, None, 5), theme);
    }
    let dark = Theme::find("faro-dark", &user).unwrap();
    state.config.theme = "faro-dark".to_owned();

    // The flood: the next events, one frame each.
    for (i, event) in demo.flood.iter().enumerate() {
        update(&mut state, AppEvent::Backend(event.clone()));
        write(format!("flood-{i:03}"), &draw(&state, dark, None, 5), dark);
    }

    // The palette over the conversation.
    key(&mut state, KeyCode::Char('p'), KeyModifiers::CONTROL);
    write("palette".to_owned(), &draw(&state, dark, None, 5), dark);
    key(&mut state, KeyCode::Esc, KeyModifiers::NONE);

    // The logo screen (nothing open), a still and a moving frame.
    while state.open.is_some() {
        key(&mut state, KeyCode::Esc, KeyModifiers::NONE);
    }
    write("logo".to_owned(), &draw(&state, dark, None, 5), dark);
    write(
        "logo-moving".to_owned(),
        &draw(&state, dark, Some(2.4), 5),
        dark,
    );

    let list = index
        .iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(",");
    std::fs::write(out.join("index.json"), format!("[{list}]")).unwrap();
}
