use super::eq::{draw_eq_summary, draw_loudness};
use super::fx::draw_fx_summary;
use super::spectrum::draw_spectrum;
use super::{ACCENT, DIM, fmt_dur};
use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};
pub(super) fn draw_now_playing(f: &mut Frame, area: Rect, app: &App) {
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
pub(super) fn draw_speed(f: &mut Frame, label_area: Rect, slider_area: Rect, app: &App) {
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
