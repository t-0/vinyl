use super::{ACCENT, DIM};
use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
pub(super) fn draw_search_box(f: &mut Frame, area: Rect, app: &App) {
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
