use super::DIM;
use crate::app::App;
use crate::spectrum::BANDS;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
pub(super) fn draw_spectrum(f: &mut Frame, area: Rect, app: &App) {
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
