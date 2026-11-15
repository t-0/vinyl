use super::{ACCENT, DIM};
use crate::app::{App, EqRow, NormalizeState};
use crate::fx::{EQ_RANGE_DB, EqMode, PREAMP_MAX_DB, PREAMP_MIN_DB, StereoMode, WIDTH_MAX};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
pub(super) fn draw_loudness(f: &mut Frame, area: Rect, app: &App) {
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
pub(super) fn draw_eq_summary(f: &mut Frame, area: Rect, app: &App) {
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
pub(super) fn draw_eq_box(f: &mut Frame, area: Rect, app: &App) {
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
