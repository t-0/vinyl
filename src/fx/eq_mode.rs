use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum EqMode {
    #[default]
    Off,
    Three,
    Seven,
}
impl EqMode {
    pub const ALL: [EqMode; 3] = [EqMode::Off, EqMode::Three, EqMode::Seven];
    pub fn label(self) -> &'static str {
        match self {
            EqMode::Off => "Off",
            EqMode::Three => "3-band",
            EqMode::Seven => "7-band",
        }
    }
    pub fn bands(self) -> usize {
        match self {
            EqMode::Off => 0,
            EqMode::Three => 3,
            EqMode::Seven => 7,
        }
    }
}
pub const THREE_BANDS: [(&str, f32); 3] = [("Low", 200.0), ("Mid", 1000.0), ("High", 4000.0)];
pub const SEVEN_BANDS: [f32; 7] = [60.0, 150.0, 400.0, 1000.0, 2500.0, 6000.0, 15000.0];
