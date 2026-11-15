use super::{ACCENT, DIM};
use crate::app::{App, FxRow};
use crate::fx::FxMode;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
pub(super) fn draw_fx_summary(f: &mut Frame, area: Rect, app: &App) {
    let p = app.fx;
    let detail = match p.mode {
        FxMode::Off => String::from("Off"),
        FxMode::Vinyl => format!(
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
    let style = if p.mode == FxMode::Off {
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
pub(super) fn draw_fx_box(f: &mut Frame, area: Rect, app: &App) {
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
            for m in FxMode::ALL {
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
        FxMode::Off => "←/→ pick a mode",
        FxMode::Vinyl => "crackle & pops, surface noise, warm asymmetric distortion",
        FxMode::ReelToReel => "wide-band tape: low hiss, gentle saturation, slow wow",
        FxMode::Cassette => "narrow-band tape: hiss, more wow & flutter, dropouts with age",
    };
    lines.push(Line::styled(hint, Style::default().fg(DIM)));
    lines.push(Line::styled(
        "↑/↓ row   ←/→ ±5   +/- ±1   0 reset   esc close",
        Style::default().fg(DIM),
    ));
    f.render_widget(Paragraph::new(lines), inner);
}
