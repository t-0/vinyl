use super::{ACCENT, DIM, fmt_dur};
use crate::app::{App, Section, SortKey};
use crate::config::display_path;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};
pub(super) fn draw_list(f: &mut Frame, area: Rect, app: &mut App) {
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
