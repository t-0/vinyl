use crate::app::{App, EqRow, FxRow, NormalizeState, Section, SortKey};
use crate::config::display_path;
use crate::fx::{EQ_RANGE_DB, EqMode, Mode, PREAMP_MAX_DB, PREAMP_MIN_DB, StereoMode, WIDTH_MAX};
use crate::spectrum::BANDS;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap};
const ACCENT: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;
pub fn fmt_dur(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let warn_lines = match &app.warning {
        Some(w) => ((w.chars().count() as u16).saturating_sub(1) / area.width.max(1) + 1).min(4),
        None => 0,
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(warn_lines),
            Constraint::Length(1),
        ])
        .split(area);
    draw_header(f, rows[0], app);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(rows[1]);
    draw_list(f, cols[0], app);
    draw_now_playing(f, cols[1], app);
    if let Some(w) = &app.warning {
        f.render_widget(
            Paragraph::new(format!(" ⚠ {w}"))
                .style(Style::default().fg(Color::Black).bg(Color::Yellow))
                .wrap(Wrap { trim: true }),
            rows[2],
        );
    }
    draw_footer(f, rows[3], app);
    if app.search_input.is_some() {
        draw_search_box(f, area, app);
    }
    if app.fx_open {
        draw_fx_box(f, area, app);
    }
    if app.eq_open {
        draw_eq_box(f, area, app);
    }
    if app.sort_open {
        draw_sort_box(f, area, app);
    }
}
fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(
            " vinyl ",
            Style::default().fg(Color::Black).bg(ACCENT).bold(),
        ),
        Span::raw("  "),
    ];
    for (key, sec) in [
        ("s", Section::Songs),
        ("a", Section::Albums),
        ("r", Section::Artists),
    ] {
        let active = app.section == sec;
        let style = if active {
            Style::default().fg(ACCENT).bold().underlined()
        } else {
            Style::default().fg(DIM)
        };
        spans.push(Span::styled(format!("[{key}] {}", sec.title()), style));
        spans.push(Span::raw("   "));
    }
    let count = match app.section {
        Section::Songs => app.view_songs.len(),
        Section::Albums => app.view_albums.len(),
        Section::Artists => app.view_artists.len(),
    };
    spans.push(Span::styled(format!("{count}"), Style::default().fg(DIM)));
    if !app.search.is_empty() {
        spans.push(Span::styled(
            format!("   / \"{}\"", app.search),
            Style::default().fg(ACCENT),
        ));
    }
    if app.sort != SortKey::Artist {
        spans.push(Span::styled(
            format!("   sort: {}", app.sort.label().to_lowercase()),
            Style::default().fg(ACCENT),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(
            format!(" {} ", app.section.title()),
            Style::default().fg(ACCENT),
        ));
    if let Some((done, total)) = app.indexing {
        let inner = block.inner(area);
        f.render_widget(block, area);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
        let label = if total == 0 {
            "scanning folders...".to_string()
        } else {
            format!("reading tags {done}/{total}")
        };
        f.render_widget(
            Paragraph::new(format!("Indexing {}", display_path(&app.config.music_home))),
            rows[0],
        );
        let ratio = if total == 0 {
            0.0
        } else {
            done as f64 / total as f64
        };
        f.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(ACCENT).bg(Color::Black))
                .ratio(ratio.clamp(0.0, 1.0))
                .label(label),
            rows[2],
        );
        return;
    }
    let inner = block.inner(area);
    let width = inner.width as usize;
    let height = (inner.height as usize).max(1);
    app.page_size = height;
    let playing = app.playing;
    let playing_album = playing.and_then(|i| app.album_of(i));
    let playing_artist = playing_album.and_then(|a| app.artist_of_album(a));
    let (len, cursor, sec) = match app.section {
        Section::Songs => (app.view_songs.len(), app.cursor_song, 0),
        Section::Albums => (app.view_albums.len(), app.cursor_album, 1),
        Section::Artists => (app.view_artists.len(), app.cursor_artist, 2),
    };
    let recenter = std::mem::take(&mut app.recenter[sec]);
    let offset = App::scroll_offset(app.offsets[sec], cursor, len, height, recenter);
    app.offsets[sec] = offset;
    let end = (offset + height).min(len);
    let window = offset..end;
    let items: Vec<ListItem> = match app.section {
        Section::Songs => app.view_songs[window]
            .iter()
            .map(|&i| (i, &app.tracks[i]))
            .map(|(i, t)| {
                let marker = if Some(i) == playing { "▶ " } else { "  " };
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
                ListItem::new(two_col(&left, &right, width, Some(i) == playing))
            })
            .collect(),
        Section::Albums => app.view_albums[window]
            .iter()
            .map(|&i| (i, &app.albums[i]))
            .map(|(i, a)| {
                let marker = if Some(i) == playing_album {
                    "▶ "
                } else {
                    "  "
                };
                let year = a.year.map(|y| format!(" ({y})")).unwrap_or_default();
                let left = format!("{marker}{} — {}{year}", a.artist, a.name);
                let n = a.end - a.start;
                let right = format!("{n} track{}", if n == 1 { "" } else { "s" });
                ListItem::new(two_col(&left, &right, width, Some(i) == playing_album))
            })
            .collect(),
        Section::Artists => app.view_artists[window]
            .iter()
            .map(|&i| (i, &app.artists[i]))
            .map(|(i, a)| {
                let marker = if Some(i) == playing_artist {
                    "▶ "
                } else {
                    "  "
                };
                let left = format!("{marker}{}", a.name);
                let na = a.album_end - a.album_start;
                let nt = a.track_end - a.track_start;
                let right = format!(
                    "{na} album{}, {nt} track{}",
                    if na == 1 { "" } else { "s" },
                    if nt == 1 { "" } else { "s" }
                );
                ListItem::new(two_col(&left, &right, width, Some(i) == playing_artist))
            })
            .collect(),
    };
    let selected = cursor.saturating_sub(offset);
    if items.is_empty() {
        let inner = block.inner(area);
        f.render_widget(block, area);
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
        f.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(DIM))
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(Color::Rgb(50, 50, 60))
            .add_modifier(Modifier::BOLD),
    );
    let mut state = ListState::default().with_selected(Some(selected));
    f.render_stateful_widget(list, area, &mut state);
}
fn two_col(left: &str, right: &str, width: usize, highlight: bool) -> Line<'static> {
    let rl = right.chars().count();
    let avail = width.saturating_sub(rl + 1);
    let mut l: String = left.chars().take(avail).collect();
    let ll = l.chars().count();
    if ll < avail {
        l.push_str(&" ".repeat(avail - ll));
    }
    let lstyle = if highlight {
        Style::default().fg(ACCENT)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::styled(l, lstyle),
        Span::raw(" "),
        Span::styled(right.to_string(), Style::default().fg(DIM)),
    ])
}
fn draw_now_playing(f: &mut Frame, area: Rect, app: &App) {
    let playing = app.playing_track();
    let paused = app.audio.as_ref().map(|a| a.is_paused()).unwrap_or(true);
    let title = match (playing.is_some(), paused) {
        (false, _) => " ⏹ Nothing Playing ",
        (true, true) => " ⏸ Paused ",
        (true, false) => " ▶ Playing ",
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(title, Style::default().fg(ACCENT).bold()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .margin(1)
        .split(inner);
    let Some(t) = playing else {
        f.render_widget(
            Paragraph::new("Select with ↑/↓, press enter to play.").style(Style::default().fg(DIM)),
            rows[0],
        );
        if let Some(err) = &app.audio_error {
            let msg = Rect {
                x: rows[2].x,
                y: rows[2].y,
                width: rows[2].width,
                height: 4,
            };
            f.render_widget(
                Paragraph::new(format!("Audio output unavailable: {err}"))
                    .style(Style::default().fg(Color::Red))
                    .wrap(Wrap { trim: true }),
                msg,
            );
        }
        draw_speed(f, rows[9], rows[10], app);
        draw_loudness(f, rows[12], app);
        draw_eq_summary(f, rows[13], app);
        draw_fx_summary(f, rows[14], app);
        draw_spectrum(f, rows[15], app);
        return;
    };
    let field = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<8}"), Style::default().fg(DIM)),
            Span::raw(value),
        ])
    };
    f.render_widget(Paragraph::new(field("Artist", t.artist.clone())), rows[0]);
    f.render_widget(
        Paragraph::new(field(
            "Track",
            match t.track {
                Some(n) => format!("{}  ({n})", t.title),
                None => t.title.clone(),
            },
        ))
        .style(Style::default().bold()),
        rows[1],
    );
    f.render_widget(Paragraph::new(field("Album", t.album.clone())), rows[2]);
    f.render_widget(
        Paragraph::new(field(
            "Year",
            t.year.map(|y| y.to_string()).unwrap_or_else(|| "—".into()),
        )),
        rows[3],
    );
    f.render_widget(
        Paragraph::new(field("Length", fmt_dur(t.duration_secs))),
        rows[4],
    );
    f.render_widget(
        Paragraph::new(field("Type", t.filetype.to_uppercase())),
        rows[5],
    );
    let pos = app
        .audio
        .as_ref()
        .map(|a| a.position().as_secs_f64())
        .unwrap_or(0.0);
    let ratio = if t.duration_secs > 0.0 {
        (pos / t.duration_secs).clamp(0.0, 1.0)
    } else {
        0.0
    };
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(ACCENT).bg(Color::Rgb(40, 40, 40)))
            .ratio(ratio)
            .label(format!("{} / {}", fmt_dur(pos), fmt_dur(t.duration_secs))),
        rows[7],
    );
    draw_speed(f, rows[9], rows[10], app);
    draw_loudness(f, rows[12], app);
    draw_eq_summary(f, rows[13], app);
    draw_fx_summary(f, rows[14], app);
    draw_spectrum(f, rows[15], app);
    f.render_widget(
        Paragraph::new(app.config.library_path(&t.path)).style(Style::default().fg(DIM)),
        rows[rows.len() - 1],
    );
}
fn draw_speed(f: &mut Frame, label_area: Rect, slider_area: Rect, app: &App) {
    let pct = app.speed_pct;
    let label = Line::from(vec![
        Span::styled("Speed   ", Style::default().fg(DIM)),
        Span::styled(
            format!("{}%", 100 + pct),
            Style::default().fg(ACCENT).bold(),
        ),
        Span::styled(
            format!("  ({:.2}x)   +/- 1%, 0 resets to 100%", app.speed()),
            Style::default().fg(DIM),
        ),
    ]);
    f.render_widget(Paragraph::new(label), label_area);
    let w = slider_area.width as usize;
    if w < 12 {
        return;
    }
    let track_w = w - 10;
    let knob = ((pct + 50) as f64 / 150.0 * (track_w - 1) as f64).round() as usize;
    let center = (50.0 / 150.0 * (track_w - 1) as f64).round() as usize;
    let mut spans = vec![Span::styled(" 50% ", Style::default().fg(DIM))];
    for i in 0..track_w {
        if i == knob {
            spans.push(Span::styled("●", Style::default().fg(ACCENT).bold()));
        } else if i == center {
            spans.push(Span::styled("┼", Style::default().fg(Color::Gray)));
        } else {
            spans.push(Span::styled("─", Style::default().fg(DIM)));
        }
    }
    spans.push(Span::styled(" 200%", Style::default().fg(DIM)));
    f.render_widget(Paragraph::new(Line::from(spans)), slider_area);
}
fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let show_status = !app.status.is_empty() && app.status_at.elapsed().as_secs() < 6;
    let line = if show_status {
        Line::from(Span::styled(
            format!(" {}", app.status),
            Style::default().fg(ACCENT),
        ))
    } else {
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
        let mut spans = Vec::new();
        for (k, d) in keys {
            if k == "E" && !editor {
                continue;
            }
            spans.push(Span::styled(
                format!(" {k} "),
                Style::default().fg(Color::Black).bg(Color::Gray),
            ));
            spans.push(Span::styled(format!(" {d}  "), Style::default().fg(DIM)));
        }
        Line::from(spans)
    };
    f.render_widget(Paragraph::new(line), area);
}
fn draw_search_box(f: &mut Frame, area: Rect, app: &App) {
    let Some(input) = &app.search_input else {
        return;
    };
    let w = area.width.clamp(20, 70);
    let h = 5;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 3;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(" Search ", Style::default().fg(ACCENT).bold()));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    let shown: String = {
        let max = inner.width.saturating_sub(4) as usize;
        let n = input.chars().count();
        input.chars().skip(n.saturating_sub(max)).collect()
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" / ", Style::default().fg(ACCENT)),
            Span::raw(shown),
            Span::styled("▏", Style::default().fg(ACCENT)),
        ])),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(" artist, album or title   enter: search   empty: show all   esc: cancel")
            .style(Style::default().fg(DIM)),
        rows[2],
    );
}
fn draw_fx_summary(f: &mut Frame, area: Rect, app: &App) {
    let p = app.fx;
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
    let style = if p.mode == Mode::Off {
        Style::default().fg(DIM)
    } else {
        Style::default().fg(ACCENT)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("FX      ", Style::default().fg(DIM)),
            Span::styled(detail, style),
        ])),
        area,
    );
}
fn draw_fx_box(f: &mut Frame, area: Rect, app: &App) {
    let rows_def = app.fx_rows();
    let w = area.width.clamp(30, 64);
    let h = (rows_def.len() as u16 + 5).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 3;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(" FX ", Style::default().fg(ACCENT).bold()));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines: Vec<Line> = Vec::new();
    let bar_w = inner.width.saturating_sub(26) as usize;
    for (i, row) in rows_def.iter().enumerate() {
        let selected = i == app.fx_row;
        let marker = if selected { "▸ " } else { "  " };
        let label_style = if selected {
            Style::default().fg(ACCENT).bold()
        } else {
            Style::default()
        };
        let mut spans = vec![Span::styled(
            format!("{marker}{:<12}", row.label()),
            label_style,
        )];
        if *row == FxRow::Mode {
            for m in Mode::ALL {
                let st = if m == app.fx.mode {
                    Style::default().fg(Color::Black).bg(ACCENT)
                } else {
                    Style::default().fg(DIM)
                };
                spans.push(Span::styled(format!(" {} ", m.label()), st));
                spans.push(Span::raw(" "));
            }
        } else {
            let v = app.fx_value(*row) as usize;
            let filled = (v * bar_w).div_ceil(100).min(bar_w);
            spans.push(Span::styled(
                "█".repeat(filled),
                Style::default().fg(ACCENT),
            ));
            spans.push(Span::styled(
                "░".repeat(bar_w - filled),
                Style::default().fg(DIM),
            ));
            spans.push(Span::raw(format!(" {v:>3}%")));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    let hint = match app.fx.mode {
        Mode::Off => "←/→ pick a mode",
        Mode::Vinyl => "crackle & pops, surface noise, warm asymmetric distortion",
        Mode::ReelToReel => "wide-band tape: low hiss, gentle saturation, slow wow",
        Mode::Cassette => "narrow-band tape: hiss, more wow & flutter, dropouts with age",
    };
    lines.push(Line::styled(hint, Style::default().fg(DIM)));
    lines.push(Line::styled(
        "↑/↓ row   ←/→ ±5   +/- ±1   0 reset   esc close",
        Style::default().fg(DIM),
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
fn draw_loudness(f: &mut Frame, area: Rect, app: &App) {
    let loud = if app.fx.loudness {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(DIM)
    };
    let stereo = match app.fx.stereo {
        StereoMode::Normal => Span::styled("Stereo", Style::default().fg(DIM)),
        StereoMode::Mono => Span::styled("Mono", Style::default().fg(ACCENT)),
        StereoMode::Width => Span::styled(
            format!("Wide {}%", app.fx.width),
            Style::default().fg(ACCENT),
        ),
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("        "),
            stereo,
            Span::raw("  "),
            Span::styled("Loudness", loud),
        ])),
        area,
    );
}
fn draw_eq_summary(f: &mut Frame, area: Rect, app: &App) {
    let eq = app.fx.eq;
    let mut spans = vec![Span::styled("EQ      ", Style::default().fg(DIM))];
    spans.push(Span::styled("Pre ", Style::default().fg(DIM)));
    spans.push(Span::styled(
        format!("{:+}dB", app.fx.preamp),
        Style::default().fg(if app.fx.preamp == 0 { DIM } else { ACCENT }),
    ));
    match app.normalize_state() {
        NormalizeState::Off => {}
        NormalizeState::Idle => spans.push(Span::styled(" auto", Style::default().fg(ACCENT))),
        NormalizeState::Measuring => {
            spans.push(Span::styled(" auto …", Style::default().fg(ACCENT)))
        }
        NormalizeState::Gain(g) => spans.push(Span::styled(
            format!(" auto {g:+.1}"),
            Style::default().fg(ACCENT),
        )),
    }
    spans.push(Span::styled("  ", Style::default().fg(DIM)));
    match eq.mode {
        EqMode::Off => spans.push(Span::styled("Off", Style::default().fg(DIM))),
        _ => {
            let gains: Vec<String> = (0..eq.mode.bands())
                .map(|b| format!("{:+}", eq.gain(b)))
                .collect();
            spans.push(Span::styled(
                format!("{}  {}", eq.mode.label(), gains.join(" ")),
                Style::default().fg(ACCENT),
            ));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}
fn draw_eq_box(f: &mut Frame, area: Rect, app: &App) {
    let eq = app.fx.eq;
    let rows = app.eq_rows();
    let w = area.width.clamp(30, 64);
    let h = (rows.len() as u16 + 5).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 3;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " Equaliser & Stereo ",
            Style::default().fg(ACCENT).bold(),
        ));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let bar_w = (inner.width.saturating_sub(26) as usize) | 1;
    let half = bar_w / 2;
    let chip = |label: &str, on: bool| {
        let st = if on {
            Style::default().fg(Color::Black).bg(ACCENT)
        } else {
            Style::default().fg(DIM)
        };
        Span::styled(format!(" {label} "), st)
    };
    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let selected = i == app.eq_row;
        let marker = if selected { "▸ " } else { "  " };
        let lstyle = if selected {
            Style::default().fg(ACCENT).bold()
        } else {
            Style::default()
        };
        let label = match row {
            EqRow::Preamp => "Preamp".to_string(),
            EqRow::Normalize => "Normalise".to_string(),
            EqRow::Mode => "Mode".to_string(),
            EqRow::Band(b) => eq.band_label(*b),
            EqRow::Loudness => "Loudness".to_string(),
            EqRow::Stereo => "Stereo".to_string(),
            EqRow::Width => "Width".to_string(),
        };
        let mut spans = vec![Span::styled(format!("{marker}{label:<14}"), lstyle)];
        match row {
            EqRow::Preamp => {
                // The range is lopsided (−24..+12), so the 0 dB mark sits at
                // two thirds and the bar fills away from it either way.
                let g = app.fx.preamp as i32;
                let span = (PREAMP_MAX_DB as i32 - PREAMP_MIN_DB as i32) as usize;
                let zero = ((-PREAMP_MIN_DB as usize) * (bar_w - 1)) / span;
                let cell = |db: i32| ((db - PREAMP_MIN_DB as i32) as usize * (bar_w - 1)) / span;
                let (lo, hi) = if g >= 0 {
                    (zero, cell(g))
                } else {
                    (cell(g), zero)
                };
                for i in 0..bar_w {
                    let (ch, st) = if i == zero {
                        ("┃", Style::default().fg(Color::Gray))
                    } else if i >= lo && i <= hi && g != 0 {
                        ("█", Style::default().fg(ACCENT))
                    } else {
                        ("░", Style::default().fg(DIM))
                    };
                    spans.push(Span::styled(ch, st));
                }
                spans.push(Span::raw(format!(" {g:+3} dB")));
            }
            EqRow::Normalize => {
                let on = app.fx.auto_normalize;
                spans.push(chip(if on { "On" } else { "Off" }, on));
                let note = match app.normalize_state() {
                    NormalizeState::Off => "  level each track, added to the preamp".to_string(),
                    NormalizeState::Idle => "  measured on first play".to_string(),
                    NormalizeState::Measuring => "  measuring this track…".to_string(),
                    NormalizeState::Gain(g) => {
                        format!(
                            "  this track {g:+.1} dB → preamp {:+.1} dB",
                            app.fx.effective_preamp_db()
                        )
                    }
                };
                spans.push(Span::styled(note, Style::default().fg(DIM)));
            }
            EqRow::Mode => {
                for m in EqMode::ALL {
                    spans.push(chip(m.label(), m == eq.mode));
                    spans.push(Span::raw(" "));
                }
            }
            EqRow::Band(b) => {
                let g = eq.gain(*b);
                let len = ((g.unsigned_abs() as usize) * half)
                    .div_ceil(EQ_RANGE_DB as usize)
                    .min(half);
                for i in 0..bar_w {
                    let on = if g > 0 {
                        i > half && i <= half + len
                    } else {
                        i < half && i + len >= half
                    };
                    let (ch, st) = if i == half {
                        ("┃", Style::default().fg(Color::Gray))
                    } else if on {
                        ("█", Style::default().fg(ACCENT))
                    } else {
                        ("░", Style::default().fg(DIM))
                    };
                    spans.push(Span::styled(ch, st));
                }
                spans.push(Span::raw(format!(" {g:+3} dB")));
            }
            EqRow::Loudness => {
                spans.push(chip(
                    if app.fx.loudness { "On" } else { "Off" },
                    app.fx.loudness,
                ));
                spans.push(Span::styled(
                    "  bass & treble lift for quiet listening",
                    Style::default().fg(DIM),
                ));
            }
            EqRow::Stereo => {
                for m in StereoMode::ALL {
                    spans.push(chip(m.label(), m == app.fx.stereo));
                    spans.push(Span::raw(" "));
                }
                spans.push(Span::styled(" mid/side", Style::default().fg(DIM)));
            }
            EqRow::Width => {
                let v = app.fx.width as usize;
                let filled = (v * bar_w).div_ceil(WIDTH_MAX as usize).min(bar_w);
                let centre = (100 * bar_w) / WIDTH_MAX as usize;
                for i in 0..bar_w {
                    let (ch, st) = if i == centre {
                        ("┃", Style::default().fg(Color::Gray))
                    } else if i < filled {
                        ("█", Style::default().fg(ACCENT))
                    } else {
                        ("░", Style::default().fg(DIM))
                    };
                    spans.push(Span::styled(ch, st));
                }
                spans.push(Span::raw(format!(" {v:>3}%")));
            }
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "↑/↓ row   ←/→ adjust   pgup/pgdn coarse   0 reset   esc close",
        Style::default().fg(DIM),
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
fn draw_sort_box(f: &mut Frame, area: Rect, app: &App) {
    let w = area.width.clamp(30, 64);
    let h = (SortKey::ALL.len() as u16 + 4).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 3;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    f.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(Span::styled(
            " Sort songs by ",
            Style::default().fg(ACCENT).bold(),
        ));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let mut lines: Vec<Line> = Vec::new();
    for (i, key) in SortKey::ALL.iter().enumerate() {
        let selected = i == app.sort_row;
        let current = *key == app.sort;
        let marker = if selected { "▸ " } else { "  " };
        let st = if selected {
            Style::default().fg(ACCENT).bold()
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{:<14}", key.label()), st),
            Span::styled(
                if current { "● " } else { "  " },
                Style::default().fg(ACCENT),
            ),
            Span::styled(key.describe(), Style::default().fg(DIM)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "↑/↓ select  enter apply  a/t/b/r shortcuts  esc cancel",
        Style::default().fg(DIM),
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
fn draw_spectrum(f: &mut Frame, area: Rect, app: &App) {
    let w = area.width as usize;
    let h = area.height as usize;
    if w < 4 || h < 2 {
        return;
    }
    let h = h - 1;
    let top = area.y + 1;
    let buf = f.buffer_mut();
    let levels = &app.analyser.levels;
    let peaks = &app.analyser.peaks;
    let bars = w.div_ceil(2);
    for bar in 0..bars {
        let x = bar * 2;
        let b0 = bar * BANDS / bars;
        let b1 = ((bar + 1) * BANDS / bars).max(b0 + 1).min(BANDS);
        let lv = levels[b0..b1].iter().cloned().fold(0.0f32, f32::max);
        let pk = peaks[b0..b1].iter().cloned().fold(0.0f32, f32::max);
        let eighths = (lv * (h * 8) as f32).round() as usize;
        let peak_row = ((pk * h as f32).ceil() as usize).clamp(1, h) - 1;
        for row in 0..h {
            let y = top + (h - 1 - row) as u16;
            let fill = eighths.saturating_sub(row * 8).min(8);
            let frac = row as f32 / h as f32;
            let color = if frac > 0.85 {
                Color::Red
            } else if frac > 0.6 {
                Color::Yellow
            } else {
                Color::Green
            };
            let (ch, fg) = if fill > 0 {
                (BLOCKS[fill], color)
            } else if row == peak_row && pk > 0.02 {
                ('▁', Color::Gray)
            } else {
                (' ', DIM)
            };
            if let Some(cell) = buf.cell_mut((area.x + x as u16, y)) {
                cell.set_char(ch);
                cell.set_fg(fg);
            }
        }
    }
}
