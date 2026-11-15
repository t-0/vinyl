mod eq;
mod footer;
mod fx;
mod header;
mod list;
mod now_playing;
mod search_box;
mod sort_box;
mod spectrum;
use crate::app::App;
use eq::draw_eq_box;
use footer::draw_footer;
use fx::draw_fx_box;
use header::draw_header;
use list::draw_list;
use now_playing::draw_now_playing;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Paragraph, Wrap};
use search_box::draw_search_box;
use sort_box::draw_sort_box;
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
