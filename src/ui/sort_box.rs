use super::{ACCENT, DIM};
use crate::app::{App, SortKey};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
pub(super) fn draw_sort_box(f: &mut Frame, area: Rect, app: &App) {
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
