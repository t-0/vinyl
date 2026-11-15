use super::{ACCENT, DIM};
use crate::app::App;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
pub(super) fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
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
