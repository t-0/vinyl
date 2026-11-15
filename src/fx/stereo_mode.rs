use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum StereoMode {
    #[default]
    Normal,
    Mono,
    Width,
}
impl StereoMode {
    pub const ALL: [StereoMode; 3] = [StereoMode::Normal, StereoMode::Mono, StereoMode::Width];
    pub fn label(self) -> &'static str {
        match self {
            StereoMode::Normal => "Normal",
            StereoMode::Mono => "Mono",
            StereoMode::Width => "Width",
        }
    }
}
pub const WIDTH_MAX: u8 = 200;
