use super::{EqMode, SEVEN_BANDS, THREE_BANDS};
use serde::{Deserialize, Serialize};
pub const EQ_RANGE_DB: i8 = 12;
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub struct EqParams {
    pub mode: EqMode,
    pub three: [i8; 3],
    pub seven: [i8; 7],
}
impl EqParams {
    pub fn gain(&self, band: usize) -> i8 {
        match self.mode {
            EqMode::Off => 0,
            EqMode::Three => self.three.get(band).copied().unwrap_or(0),
            EqMode::Seven => self.seven.get(band).copied().unwrap_or(0),
        }
    }
    pub fn gain_mut(&mut self, band: usize) -> Option<&mut i8> {
        match self.mode {
            EqMode::Off => None,
            EqMode::Three => self.three.get_mut(band),
            EqMode::Seven => self.seven.get_mut(band),
        }
    }
    pub fn band_label(&self, band: usize) -> String {
        match self.mode {
            EqMode::Off => String::new(),
            EqMode::Three => format!(
                "{:<5}{:>5} Hz",
                THREE_BANDS[band].0, THREE_BANDS[band].1 as u32
            ),
            EqMode::Seven => {
                let f = SEVEN_BANDS[band];
                if f >= 1000.0 {
                    format!("{:>7.1} kHz", f / 1000.0)
                } else {
                    format!("{:>8} Hz", f as u32)
                }
            }
        }
    }
}
