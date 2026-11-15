use super::{ACCENT, DIM};
use crate::app::{App, Section, SortKey};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
pub(super) fn draw_header(f: &mut Frame, area: Rect, app: &App) {
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
