use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum FxMode {
    #[default]
    Off,
    Vinyl,
    ReelToReel,
    Cassette,
}
impl FxMode {
    pub const ALL: [FxMode; 4] = [
        FxMode::Off,
        FxMode::Vinyl,
        FxMode::ReelToReel,
        FxMode::Cassette,
    ];
    pub fn label(self) -> &'static str {
        match self {
            FxMode::Off => "Off",
            FxMode::Vinyl => "Vinyl",
            FxMode::ReelToReel => "Reel-to-reel",
            FxMode::Cassette => "Cassette",
        }
    }
    pub fn is_tape(self) -> bool {
        matches!(self, FxMode::ReelToReel | FxMode::Cassette)
    }
}
