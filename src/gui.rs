//! The desktop front end (`vinyl --gui`): a GPUI window laid out like the
//! terminal UI in [`crate::ui`], with the same keys (see [`crate::keys`]).
//!
//! The screen is one monospace "page": a header line, a list panel and a
//! Now Playing panel side by side, an optional warning strip and a footer of
//! key hints, with the search / FX / EQ / sort dialogs drawn over the top.
use crate::app::{App, EqRow, FxRow, NormalizeState, Section, SortKey};
use crate::config::{Config, display_path};
use crate::fx::{EQ_RANGE_DB, EqMode, Mode, PREAMP_MAX_DB, PREAMP_MIN_DB, StereoMode, WIDTH_MAX};
use crate::keys::{self, Key};
use crate::spectrum::BANDS;
use crate::ui::fmt_dur;
use anyhow::{Result, anyhow};
use gpui::{
    AnyElement, Application, Bounds, Context, Div, FocusHandle, FontWeight, Hsla, KeyBinding,
    KeyDownEvent, Menu, MenuItem, MouseButton, MouseDownEvent, ScrollDelta, ScrollWheelEvent,
    SharedString, TitlebarOptions, Window, WindowBounds, WindowOptions, actions, canvas, div, fill,
    point, prelude::*, px, relative, rgb, size,
};
use std::process::ExitStatus;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

// The terminal palette, pinned to concrete colours.
const BG: u32 = 0x101014;
const FG: u32 = 0xd8d8d8;
const ACCENT: u32 = 0xf2c744;
const DIM: u32 = 0x74747e;
const GRAY: u32 = 0x9c9ca4;
const BLACK: u32 = 0x101014;
const RED: u32 = 0xe06c75;
const GREEN: u32 = 0x7ec16e;
const SELECT_BG: u32 = 0x32323c;
const TRACK_BG: u32 = 0x2a2a30;
const GAUGE_BG: u32 = 0x282828;
/// One text row; every line of the layout is this tall, like a terminal cell.
const ROW: f32 = 20.0;
const FONT_SIZE: f32 = 13.0;

fn col(c: u32) -> Hsla {
    rgb(c).into()
}

// macOS has no way to quit an app without a menu, so give it one entry.
actions!(vinyl, [Quit]);

pub fn run(config: Config) -> Result<()> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        return Err(anyhow!(
            "--gui needs a desktop session (neither WAYLAND_DISPLAY nor DISPLAY is set)"
        ));
    }
    let app = App::new(config);
    Application::new().run(move |cx| {
        let font = pick_font(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu {
            name: "vinyl".into(),
            items: vec![MenuItem::action("Quit vinyl", Quit)],
        }]);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("vinyl".into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(720.), px(480.))),
            app_id: Some("vinyl".into()),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            cx.new(|cx| Vinyl::new(app, font, window, cx))
        })
        .expect("open the vinyl window");
        cx.activate(true);
    });
    Ok(())
}

/// The first installed monospace family from a per-platform shortlist, so the
/// window reads like the terminal does.
fn pick_font(cx: &gpui::App) -> SharedString {
    let installed = cx.text_system().all_font_names();
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["Menlo", "SF Mono", "Monaco", "Courier New"]
    } else if cfg!(target_os = "windows") {
        &["Cascadia Mono", "Consolas", "Courier New"]
    } else {
        &[
            "DejaVu Sans Mono",
            "Ubuntu Mono",
            "Liberation Mono",
            "Noto Sans Mono",
            "JetBrains Mono",
            "Fira Mono",
        ]
    };
    candidates
        .iter()
        .find(|c| installed.iter().any(|f| f == *c))
        .map(|c| SharedString::from(c.to_string()))
        .unwrap_or_else(|| "monospace".into())
}

struct Vinyl {
    app: App,
    focus: FocusHandle,
    font: SharedString,
    /// A terminal editor started from the window runs in the background
    /// (there is no screen to hand over); its exit status arrives here.
    editor_rx: Option<(String, Receiver<std::io::Result<ExitStatus>>)>,
}

impl Vinyl {
    fn new(app: App, font: SharedString, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(wait) = this.update(cx, |v, cx| v.on_timer(cx)) else {
                    break;
                };
                cx.background_executor().timer(wait).await;
            }
        })
        .detach();
        Self {
            app,
            focus,
            font,
            editor_rx: None,
        }
    }

    /// The periodic update the terminal loop does between key presses.
    fn on_timer(&mut self, cx: &mut Context<Self>) -> Duration {
        self.app.update_spectrum();
        if let Some((name, rx)) = &self.editor_rx
            && let Ok(result) = rx.try_recv()
        {
            let name = name.clone();
            self.editor_rx = None;
            self.app.editor_finished(&name, result);
        }
        if let Err(e) = self.app.tick() {
            self.app.set_status(format!("{e:#}"));
        }
        self.after_input(cx);
        if self.app.audio_active() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(200)
        }
    }

    /// Housekeeping after anything that may have changed state.
    fn after_input(&mut self, cx: &mut Context<Self>) {
        if self.app.should_quit {
            cx.quit();
            return;
        }
        self.app.flush_play();
        if let Some((name, mut cmd)) = self.app.pending_editor.take() {
            // A `cli` editor expects a terminal. The window does not own one,
            // so the editor gets whatever terminal vinyl was launched from and
            // is awaited on a thread rather than taking over the screen.
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(cmd.status());
            });
            self.app
                .set_status(format!("running {name} in the launching terminal…"));
            self.editor_rx = Some((name, rx));
        }
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        let key = match ks.key.as_str() {
            "escape" => Key::Esc,
            "enter" => Key::Enter,
            "backspace" => Key::Backspace,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "home" => Key::Home,
            "end" => Key::End,
            "space" => Key::Char(' '),
            name => {
                let typed = ks
                    .key_char
                    .as_deref()
                    .filter(|_| !ctrl)
                    .and_then(single_char)
                    .or_else(|| single_char(name));
                match typed {
                    Some(c) => Key::Char(c),
                    None => return,
                }
            }
        };
        keys::handle(&mut self.app, key, ctrl);
        self.after_input(cx);
    }

    fn set_cursor(&mut self, index: usize) {
        match self.app.section {
            Section::Songs => self.app.cursor_song = index,
            Section::Albums => self.app.cursor_album = index,
            Section::Artists => self.app.cursor_artist = index,
        }
    }

    fn on_list_scroll(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let lines = match ev.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => f32::from(p.y) / ROW,
        };
        let delta = (-lines).round() as i64;
        if delta != 0 {
            self.app.move_cursor(delta);
            self.after_input(cx);
        }
    }
}

fn single_char(s: &str) -> Option<char> {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// A one-row flex line.
fn row() -> Div {
    div()
        .h(px(ROW))
        .flex()
        .flex_row()
        .items_center()
        .whitespace_nowrap()
        .overflow_hidden()
}

fn text(s: impl Into<SharedString>, c: u32) -> Div {
    div().text_color(col(c)).whitespace_nowrap().child(s.into())
}

fn bold(s: impl Into<SharedString>, c: u32) -> Div {
    text(s, c).font_weight(FontWeight::BOLD)
}

/// `label` on a `bg` block, like a reverse-video span.
fn chip(label: impl Into<SharedString>, fg: u32, bg: u32) -> Div {
    div()
        .px(px(4.))
        .bg(col(bg))
        .text_color(col(fg))
        .whitespace_nowrap()
        .child(label.into())
}

/// A bordered box with its title sitting on the top edge, like a ratatui
/// block.
fn panel(title: impl Into<SharedString>, border: u32, title_color: u32) -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .border_1()
        .border_color(col(border))
        .child(
            div()
                .absolute()
                .top(px(-ROW / 2.))
                .left(px(8.))
                .px(px(4.))
                .bg(col(BG))
                .text_color(col(title_color))
                .whitespace_nowrap()
                .child(title.into()),
        )
}

/// A horizontal bar filled from the left: `ratio` of its width in `fg`.
fn bar(ratio: f32, fg: u32, bg: u32) -> Div {
    let ratio = ratio.clamp(0.0, 1.0);
    div().flex_1().h(px(ROW - 6.)).relative().bg(col(bg)).child(
        div()
            .absolute()
            .left_0()
            .top_0()
            .bottom_0()
            .w(relative(ratio))
            .bg(col(fg)),
    )
}

/// A bar that fills away from a zero mark at `zero` (0..1) out to `value`
/// (0..1), with the mark drawn on top.
fn centred_bar(zero: f32, value: f32, fg: u32) -> Div {
    let (lo, hi) = if value >= zero {
        (zero, value)
    } else {
        (value, zero)
    };
    div()
        .flex_1()
        .h(px(ROW - 6.))
        .relative()
        .bg(col(TRACK_BG))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(lo))
                .w(relative((hi - lo).max(0.0)))
                .bg(col(fg)),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(zero))
                .w(px(2.))
                .bg(col(GRAY)),
        )
}

impl Render for Vinyl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width);
        let height = f32::from(viewport.height);
        // Roughly how many lines a warning wraps to, so the list below it can
        // size itself: a monospace glyph is about 0.6 em wide.
        let warn_rows = match &self.app.warning {
            Some(w) => {
                let chars_per_line = (width / (FONT_SIZE * 0.6)).max(1.0);
                ((w.chars().count() as f32 / chars_per_line).ceil() as usize).clamp(1, 4)
            }
            None => 0,
        };
        // Header, footer, half a row for the panel titles, two border pixels
        // and the panel padding leave this much for list rows.
        let body_h = height - ROW * 2.0 - ROW / 2.0 - ROW * warn_rows as f32;
        let list_rows = ((body_h - 2.0 - 4.0) / ROW).floor().max(1.0) as usize;
        self.app.page_size = list_rows;

        let mut page = div()
            .track_focus(&self.focus)
            .key_context("Vinyl")
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, _| window.focus(&this.focus)),
            )
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(col(BG))
            .text_color(col(FG))
            .font_family(self.font.clone())
            .text_size(px(FONT_SIZE))
            .line_height(px(ROW))
            .child(self.header(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .pt(px(ROW / 2.))
                    .flex()
                    .flex_row()
                    .gap(px(4.))
                    .child(self.list_panel(list_rows, cx))
                    .child(self.now_playing_panel()),
            );
        if let Some(w) = &self.app.warning {
            page = page.child(
                div()
                    .w_full()
                    .px(px(4.))
                    .bg(col(ACCENT))
                    .text_color(col(BLACK))
                    .child(format!("⚠ {w}")),
            );
        }
        page = page.child(self.footer());
        if let Some(popup) = self.popup(height) {
            page = page.child(popup);
        }
        page
    }
}

impl Vinyl {
    fn header(&self, cx: &mut Context<Self>) -> Div {
        let app = &self.app;
        let mut line = row()
            .px(px(4.))
            .gap(px(12.))
            .child(chip("vinyl", BLACK, ACCENT).font_weight(FontWeight::BOLD));
        for (key, sec) in [
            ("s", Section::Songs),
            ("a", Section::Albums),
            ("r", Section::Artists),
        ] {
            let active = app.section == sec;
            let label = format!("[{key}] {}", sec.title());
            let tab = if active {
                bold(label, ACCENT).underline()
            } else {
                text(label, DIM)
            };
            line = line.child(tab.cursor_pointer().on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                    this.app.section = sec;
                    this.after_input(cx);
                }),
            ));
        }
        let count = match app.section {
            Section::Songs => app.view_songs.len(),
            Section::Albums => app.view_albums.len(),
            Section::Artists => app.view_artists.len(),
        };
        line = line.child(text(count.to_string(), DIM));
        if !app.search.is_empty() {
            line = line.child(text(format!("/ \"{}\"", app.search), ACCENT));
        }
        if app.sort != SortKey::Artist {
            line = line.child(text(
                format!("sort: {}", app.sort.label().to_lowercase()),
                ACCENT,
            ));
        }
        line
    }

    fn list_panel(&mut self, rows: usize, cx: &mut Context<Self>) -> Div {
        let app = &mut self.app;
        let panel = panel(format!(" {} ", app.section.title()), DIM, ACCENT)
            .w(relative(0.55))
            .min_w_0()
            .h_full()
            .p(px(2.));
        if let Some((done, total)) = app.indexing {
            let label = if total == 0 {
                "scanning folders...".to_string()
            } else {
                format!("reading tags {done}/{total}")
            };
            let ratio = if total == 0 {
                0.0
            } else {
                done as f32 / total as f32
            };
            return panel
                .px(px(6.))
                .child(row().child(format!("Indexing {}", display_path(&app.config.music_home))))
                .child(row())
                .child(gauge(ratio, label));
        }
        let playing = app.playing;
        let playing_album = playing.and_then(|i| app.album_of(i));
        let playing_artist = playing_album.and_then(|a| app.artist_of_album(a));
        let (len, cursor, sec) = match app.section {
            Section::Songs => (app.view_songs.len(), app.cursor_song, 0),
            Section::Albums => (app.view_albums.len(), app.cursor_album, 1),
            Section::Artists => (app.view_artists.len(), app.cursor_artist, 2),
        };
        if len == 0 {
            let msg = if app.tracks.is_empty() {
                format!(
                    "No audio files found under {}\nEdit ~/.vinyl to change music.home, or press i to re-index.",
                    display_path(&app.config.music_home)
                )
            } else {
                format!(
                    "Nothing matches \"{}\".\nPress / then enter to show everything again.",
                    app.search
                )
            };
            return panel.px(px(6.)).child(text(msg, DIM).whitespace_normal());
        }
        let recenter = std::mem::take(&mut app.recenter[sec]);
        let offset = App::scroll_offset(app.offsets[sec], cursor, len, rows, recenter);
        app.offsets[sec] = offset;
        let end = (offset + rows).min(len);
        let mut items: Vec<(usize, String, String, bool)> = Vec::with_capacity(end - offset);
        match app.section {
            Section::Songs => {
                for &i in &app.view_songs[offset..end] {
                    let t = &app.tracks[i];
                    let now = Some(i) == playing;
                    let marker = if now { "▶ " } else { "  " };
                    let dur = fmt_dur(t.duration_secs);
                    let (left, right) = match app.sort {
                        SortKey::Artist | SortKey::ArtistTrack => (
                            format!("{marker}{} — {}", t.artist, t.title),
                            format!("{}  {dur}", t.album),
                        ),
                        SortKey::Track => (
                            format!("{marker}{} — {}", t.title, t.artist),
                            format!("{}  {dur}", t.album),
                        ),
                        SortKey::Album => (
                            format!("{marker}{} — {}", t.album, t.title),
                            format!("{}  {dur}", t.artist),
                        ),
                    };
                    items.push((i, left, right, now));
                }
            }
            Section::Albums => {
                for &i in &app.view_albums[offset..end] {
                    let a = &app.albums[i];
                    let now = Some(i) == playing_album;
                    let marker = if now { "▶ " } else { "  " };
                    let year = a.year.map(|y| format!(" ({y})")).unwrap_or_default();
                    let n = a.end - a.start;
                    items.push((
                        i,
                        format!("{marker}{} — {}{year}", a.artist, a.name),
                        format!("{n} track{}", if n == 1 { "" } else { "s" }),
                        now,
                    ));
                }
            }
            Section::Artists => {
                for &i in &app.view_artists[offset..end] {
                    let a = &app.artists[i];
                    let now = Some(i) == playing_artist;
                    let marker = if now { "▶ " } else { "  " };
                    let na = a.album_end - a.album_start;
                    let nt = a.track_end - a.track_start;
                    items.push((
                        i,
                        format!("{marker}{}", a.name),
                        format!(
                            "{na} album{}, {nt} track{}",
                            if na == 1 { "" } else { "s" },
                            if nt == 1 { "" } else { "s" }
                        ),
                        now,
                    ));
                }
            }
        }
        let mut list = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(Self::on_list_scroll));
        for (n, (_, left, right, now)) in items.into_iter().enumerate() {
            let index = offset + n;
            let selected = index == cursor;
            let mut line = row()
                .px(px(4.))
                .gap(px(8.))
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.set_cursor(index);
                        if ev.click_count >= 2 {
                            this.app.activate_cursor();
                        }
                        this.after_input(cx);
                    }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(col(if now { ACCENT } else { FG }))
                        .child(left),
                )
                .child(text(right, DIM).flex_shrink_0());
            if selected {
                line = line.bg(col(SELECT_BG)).font_weight(FontWeight::BOLD);
            }
            list = list.child(line);
        }
        panel.child(list)
    }

    fn now_playing_panel(&self) -> Div {
        let app = &self.app;
        let wrap = |body: Div| {
            panel(" Now Playing ", DIM, ACCENT)
                .flex_1()
                .min_w_0()
                .h_full()
                .child(body)
        };
        let panel = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .px(px(10.))
            .py(px(ROW))
            .overflow_hidden();
        let field = |label: &str, value: String| {
            row()
                .child(text(format!("{label:<8}"), DIM))
                .child(div().whitespace_nowrap().text_ellipsis().child(value))
        };
        let Some(t) = app.playing_track() else {
            let mut p = panel
                .child(row().child(text("Nothing playing.", DIM)))
                .child(row().child(text("Select with ↑/↓, press enter to play.", DIM)));
            if let Some(err) = &app.audio_error {
                p = p.child(row()).child(
                    text(format!("Audio output unavailable: {err}"), RED).whitespace_normal(),
                );
            } else {
                p = p.child(row()).child(row());
            }
            return wrap(
                p.child(div().h(px(ROW * 6.)))
                    .child(row())
                    .child(self.speed_label())
                    .child(self.speed_slider())
                    .child(row())
                    .child(self.fx_summary())
                    .child(self.eq_summary())
                    .child(self.spectrum()),
            );
        };
        let paused = app.audio.as_ref().map(|a| a.is_paused()).unwrap_or(true);
        let state = if paused { "⏸ paused" } else { "▶ playing" };
        let pos = app
            .audio
            .as_ref()
            .map(|a| a.position().as_secs_f64())
            .unwrap_or(0.0);
        let ratio = if t.duration_secs > 0.0 {
            (pos / t.duration_secs).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        wrap(
            panel
                .child(row().child(bold(state, ACCENT)))
                .child(row())
                .child(field("Artist", t.artist.clone()))
                .child(
                    field(
                        "Track",
                        match t.track {
                            Some(n) => format!("{}  ({n})", t.title),
                            None => t.title.clone(),
                        },
                    )
                    .font_weight(FontWeight::BOLD),
                )
                .child(field("Album", t.album.clone()))
                .child(field(
                    "Year",
                    t.year.map(|y| y.to_string()).unwrap_or_else(|| "—".into()),
                ))
                .child(field("Length", fmt_dur(t.duration_secs)))
                .child(field("Type", t.filetype.to_uppercase()))
                .child(row())
                .child(gauge(
                    ratio,
                    format!("{} / {}", fmt_dur(pos), fmt_dur(t.duration_secs)),
                ))
                .child(row())
                .child(self.speed_label())
                .child(self.speed_slider())
                .child(row())
                .child(self.fx_summary())
                .child(self.eq_summary())
                .child(self.spectrum())
                .child(row().child(text(app.config.library_path(&t.path), DIM).truncate())),
        )
    }

    fn speed_label(&self) -> Div {
        let app = &self.app;
        row()
            .child(text("Speed   ", DIM))
            .child(bold(format!("{}%", 100 + app.speed_pct), ACCENT))
            .child(text(
                format!("  ({:.2}x)   +/- 1%, 0 resets to 100%", app.speed()),
                DIM,
            ))
    }

    fn speed_slider(&self) -> Div {
        let frac = ((self.app.speed_pct + 50) as f32 / 100.0).clamp(0.0, 1.0);
        row()
            .child(text(" 50% ", DIM))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(ROW / 2. - 1.))
                            .h(px(2.))
                            .bg(col(DIM)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(relative(0.5))
                            .ml(px(-1.))
                            .top(px(ROW / 2. - 6.))
                            .w(px(2.))
                            .h(px(12.))
                            .bg(col(GRAY)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(relative(frac))
                            .ml(px(-5.))
                            .top(px(ROW / 2. - 5.))
                            .size(px(10.))
                            .rounded_full()
                            .bg(col(ACCENT)),
                    ),
            )
            .child(text(" 150%", DIM))
    }

    fn fx_summary(&self) -> Div {
        let p = self.app.fx;
        let detail = match p.mode {
            Mode::Off => String::from("Off"),
            Mode::Vinyl => format!(
                "{}   scratch {}  distortion {}",
                p.mode.label(),
                p.scratch,
                p.distortion
            ),
            _ => format!(
                "{}   age {}  hiss {}  sat {}  wow {}  flt {}",
                p.mode.label(),
                p.age,
                p.hiss,
                p.saturation,
                p.wow,
                p.flutter
            ),
        };
        let c = if p.mode == Mode::Off { DIM } else { ACCENT };
        row().child(text("FX      ", DIM)).child(text(detail, c))
    }

    fn eq_summary(&self) -> Div {
        let app = &self.app;
        let eq = app.fx.eq;
        let mut line = row()
            .child(text("EQ      ", DIM))
            .child(text("Pre ", DIM))
            .child(text(
                format!("{:+}dB", app.fx.preamp),
                if app.fx.preamp == 0 { DIM } else { ACCENT },
            ));
        line = match app.normalize_state() {
            NormalizeState::Off => line,
            NormalizeState::Idle => line.child(text(" auto", ACCENT)),
            NormalizeState::Measuring => line.child(text(" auto …", ACCENT)),
            NormalizeState::Gain(g) => line.child(text(format!(" auto {g:+.1}"), ACCENT)),
        };
        line = line.child(text("  ", DIM));
        line = match eq.mode {
            EqMode::Off => line.child(text("Off", DIM)),
            _ => {
                let gains: Vec<String> = (0..eq.mode.bands())
                    .map(|b| format!("{:+}", eq.gain(b)))
                    .collect();
                line.child(text(
                    format!("{}  {}", eq.mode.label(), gains.join(" ")),
                    ACCENT,
                ))
            }
        };
        line = line
            .child(text("  Loudness ", DIM))
            .child(if app.fx.loudness {
                text("On", ACCENT)
            } else {
                text("Off", DIM)
            });
        line.child(text("  Stereo ", DIM))
            .child(match app.fx.stereo {
                StereoMode::Normal => text("Normal", DIM),
                StereoMode::Mono => text("Mono", ACCENT),
                StereoMode::Width => text(format!("Width {}%", app.fx.width), ACCENT),
            })
    }

    /// The analyser: bars coloured green, yellow and red by height, with a
    /// grey peak marker, painted straight into the remaining panel space.
    fn spectrum(&self) -> impl IntoElement {
        let levels = self.app.analyser.levels;
        let peaks = self.app.analyser.peaks;
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let w = f32::from(bounds.size.width);
                let h = f32::from(bounds.size.height) - ROW / 2.0;
                if w < 20.0 || h < 10.0 {
                    return;
                }
                let bars = ((w / 10.0).floor() as usize).clamp(1, BANDS);
                let slot = w / bars as f32;
                let bar_w = (slot - 2.0).max(1.0);
                let x0 = f32::from(bounds.origin.x);
                let bottom = f32::from(bounds.origin.y) + f32::from(bounds.size.height);
                let green: Hsla = col(GREEN);
                let yellow: Hsla = col(ACCENT);
                let red: Hsla = col(RED);
                let grey: Hsla = col(GRAY);
                for bar in 0..bars {
                    let b0 = bar * BANDS / bars;
                    let b1 = ((bar + 1) * BANDS / bars).max(b0 + 1).min(BANDS);
                    let lv = levels[b0..b1].iter().cloned().fold(0.0f32, f32::max);
                    let pk = peaks[b0..b1].iter().cloned().fold(0.0f32, f32::max);
                    let x = x0 + bar as f32 * slot + 1.0;
                    // Three stacked segments, each clipped to the level.
                    for (lo, hi, color) in
                        [(0.0, 0.6, green), (0.6, 0.85, yellow), (0.85, 1.0, red)]
                    {
                        let top = lv.min(hi);
                        if top <= lo {
                            continue;
                        }
                        let y_lo = bottom - lo * h;
                        let y_hi = bottom - top * h;
                        window.paint_quad(fill(
                            Bounds {
                                origin: point(px(x), px(y_hi)),
                                size: size(px(bar_w), px(y_lo - y_hi)),
                            },
                            color,
                        ));
                    }
                    if pk > 0.02 {
                        let y = bottom - pk * h;
                        window.paint_quad(fill(
                            Bounds {
                                origin: point(px(x), px(y)),
                                size: size(px(bar_w), px(2.0)),
                            },
                            grey,
                        ));
                    }
                }
            },
        )
        .flex_1()
        .min_h_0()
        .w_full()
    }

    fn footer(&self) -> Div {
        let app = &self.app;
        let show_status = !app.status.is_empty() && app.status_at.elapsed().as_secs() < 6;
        let line = row().px(px(4.)).gap(px(6.));
        if show_status {
            return line.child(text(app.status.clone(), ACCENT));
        }
        let editor = app.config.audio_editor.is_some();
        let keys = [
            ("/", "Search"),
            ("o", "Sort"),
            ("x", "FX"),
            ("e", "EQ"),
            ("l", "Loudness"),
            ("↑/↓ pgup/pgdn", "Select"),
            ("⏎", "Play"),
            ("+/-", "Speed"),
            ("space", "Play/Pause"),
            ("n/p", "Track"),
            ("z", if app.shuffle { "Shuffle on" } else { "Shuffle" }),
            ("←/→", "Scan"),
            (
                "f",
                if cfg!(target_os = "macos") {
                    "Finder"
                } else {
                    "Show File"
                },
            ),
            ("E", "Edit"),
            ("i", "Reindex"),
            ("q", "Quit"),
        ];
        let mut line = line;
        for (k, d) in keys {
            if k == "E" && !editor {
                continue;
            }
            line = line.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.))
                    .child(chip(k, BLACK, GRAY))
                    .child(text(d, DIM)),
            );
        }
        line
    }

    /// The dialog that is open, if any, centred in the upper part of the
    /// window like the terminal's popups.
    fn popup(&self, height: f32) -> Option<AnyElement> {
        let app = &self.app;
        let (title, body): (&str, Div) = if app.search_input.is_some() {
            (" Search ", self.search_box())
        } else if app.fx_open {
            (" FX ", self.fx_box())
        } else if app.eq_open {
            (" Equaliser & Stereo ", self.eq_box())
        } else if app.sort_open {
            (" Sort songs by ", self.sort_box())
        } else {
            return None;
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .pt(px((height * 0.22).max(ROW)))
                .child(
                    panel(title, ACCENT, ACCENT)
                        .font_weight(FontWeight::BOLD)
                        .w(px(620.))
                        .max_w(relative(0.95))
                        .bg(col(BG))
                        .px(px(8.))
                        .py(px(ROW / 2.))
                        .child(body.font_weight(FontWeight::NORMAL)),
                )
                .into_any_element(),
        )
    }

    fn search_box(&self) -> Div {
        let input = self.app.search_input.clone().unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .child(
                row()
                    .child(text(" / ", ACCENT))
                    .child(div().child(input))
                    .child(text("▏", ACCENT)),
            )
            .child(row())
            .child(row().child(text(
                "artist, album or title   enter: search   empty: show all   esc: cancel",
                DIM,
            )))
    }

    fn fx_box(&self) -> Div {
        let app = &self.app;
        let mut lines = div().flex().flex_col();
        for (i, r) in app.fx_rows().iter().enumerate() {
            let selected = i == app.fx_row;
            let marker = if selected { "▸ " } else { "  " };
            let label = format!("{marker}{:<12}", r.label());
            let mut line = row().gap(px(6.)).child(if selected {
                bold(label, ACCENT)
            } else {
                text(label, FG)
            });
            if *r == FxRow::Mode {
                for m in Mode::ALL {
                    line = line.child(if m == app.fx.mode {
                        chip(m.label(), BLACK, ACCENT)
                    } else {
                        chip(m.label(), DIM, BG)
                    });
                }
            } else {
                let v = app.fx_value(*r);
                line = line
                    .child(bar(v as f32 / 100.0, ACCENT, TRACK_BG))
                    .child(text(format!("{v:>3}%"), FG));
            }
            lines = lines.child(line);
        }
        let hint = match app.fx.mode {
            Mode::Off => "←/→ pick a mode",
            Mode::Vinyl => "crackle & pops, surface noise, warm asymmetric distortion",
            Mode::ReelToReel => "wide-band tape: low hiss, gentle saturation, slow wow",
            Mode::Cassette => "narrow-band tape: hiss, more wow & flutter, dropouts with age",
        };
        lines
            .child(row())
            .child(row().child(text(hint, DIM)))
            .child(row().child(text("↑/↓ row   ←/→ ±5   +/- ±1   0 reset   esc close", DIM)))
    }

    fn eq_box(&self) -> Div {
        let app = &self.app;
        let eq = app.fx.eq;
        let on_chip = |label: &str, on: bool| {
            if on {
                chip(label.to_string(), BLACK, ACCENT)
            } else {
                chip(label.to_string(), DIM, BG)
            }
        };
        let mut lines = div().flex().flex_col();
        for (i, r) in app.eq_rows().iter().enumerate() {
            let selected = i == app.eq_row;
            let marker = if selected { "▸ " } else { "  " };
            let label = match r {
                EqRow::Preamp => "Preamp".to_string(),
                EqRow::Normalize => "Normalise".to_string(),
                EqRow::Mode => "Mode".to_string(),
                EqRow::Band(b) => eq.band_label(*b),
                EqRow::Loudness => "Loudness".to_string(),
                EqRow::Stereo => "Stereo".to_string(),
                EqRow::Width => "Width".to_string(),
            };
            let label = format!("{marker}{label:<14}");
            let mut line = row().gap(px(6.)).child(if selected {
                bold(label, ACCENT)
            } else {
                text(label, FG)
            });
            match r {
                EqRow::Preamp => {
                    // The range is lopsided (−24..+12), so the 0 dB mark sits
                    // at two thirds and the bar fills away from it.
                    let span = (PREAMP_MAX_DB as f32 - PREAMP_MIN_DB as f32).max(1.0);
                    let zero = -PREAMP_MIN_DB as f32 / span;
                    let value = (app.fx.preamp as f32 - PREAMP_MIN_DB as f32) / span;
                    line = line
                        .child(centred_bar(zero, value, ACCENT))
                        .child(text(format!("{:+3} dB", app.fx.preamp), FG));
                }
                EqRow::Normalize => {
                    let on = app.fx.auto_normalize;
                    let note = match app.normalize_state() {
                        NormalizeState::Off => {
                            "  level each track, added to the preamp".to_string()
                        }
                        NormalizeState::Idle => "  measured on first play".to_string(),
                        NormalizeState::Measuring => "  measuring this track…".to_string(),
                        NormalizeState::Gain(g) => format!(
                            "  this track {g:+.1} dB → preamp {:+.1} dB",
                            app.fx.effective_preamp_db()
                        ),
                    };
                    line = line
                        .child(on_chip(if on { "On" } else { "Off" }, on))
                        .child(text(note, DIM));
                }
                EqRow::Mode => {
                    for m in EqMode::ALL {
                        line = line.child(on_chip(m.label(), m == eq.mode));
                    }
                }
                EqRow::Band(b) => {
                    let g = eq.gain(*b);
                    let value = 0.5 + (g as f32 / EQ_RANGE_DB as f32) * 0.5;
                    line = line
                        .child(centred_bar(0.5, value, ACCENT))
                        .child(text(format!("{g:+3} dB"), FG));
                }
                EqRow::Loudness => {
                    line = line
                        .child(on_chip(
                            if app.fx.loudness { "On" } else { "Off" },
                            app.fx.loudness,
                        ))
                        .child(text("  bass & treble lift for quiet listening", DIM));
                }
                EqRow::Stereo => {
                    for m in StereoMode::ALL {
                        line = line.child(on_chip(m.label(), m == app.fx.stereo));
                    }
                    line = line.child(text(" mid/side", DIM));
                }
                EqRow::Width => {
                    let v = app.fx.width;
                    line = line
                        .child(centred_bar(
                            100.0 / WIDTH_MAX as f32,
                            v as f32 / WIDTH_MAX as f32,
                            ACCENT,
                        ))
                        .child(text(format!("{v:>3}%"), FG));
                }
            }
            lines = lines.child(line);
        }
        lines.child(row()).child(row().child(text(
            "↑/↓ row   ←/→ adjust   pgup/pgdn coarse   0 reset   esc close",
            DIM,
        )))
    }

    fn sort_box(&self) -> Div {
        let app = &self.app;
        let mut lines = div().flex().flex_col();
        for (i, key) in SortKey::ALL.iter().enumerate() {
            let selected = i == app.sort_row;
            let current = *key == app.sort;
            let marker = if selected { "▸ " } else { "  " };
            let label = format!("{marker}{:<14}", key.label());
            lines = lines.child(
                row()
                    .child(if selected {
                        bold(label, ACCENT)
                    } else {
                        text(label, FG)
                    })
                    .child(text(if current { "● " } else { "  " }, ACCENT))
                    .child(text(key.describe(), DIM)),
            );
        }
        lines.child(row()).child(row().child(text(
            "↑/↓ select  enter apply  a/t/b/r shortcuts  esc cancel",
            DIM,
        )))
    }
}

/// A progress bar with its label centred on top, like a ratatui gauge.
fn gauge(ratio: f32, label: String) -> Div {
    let ratio = ratio.clamp(0.0, 1.0);
    // The label sits mostly over the filled part once it passes the middle,
    // so flip its colour to stay readable.
    let label_color = if ratio > 0.5 { BLACK } else { FG };
    div()
        .h(px(ROW))
        .relative()
        .bg(col(GAUGE_BG))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(relative(ratio))
                .bg(col(ACCENT)),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .text_color(col(label_color))
                .child(label),
        )
}
