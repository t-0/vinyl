use super::{EqParams, FxMode, StereoMode};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
/// Preamp gain range in dB. It sits in front of the whole chain, so a boost
/// drives the tape saturation and vinyl distortion harder and a cut backs
/// them off.
pub const PREAMP_MIN_DB: i8 = -24;
pub const PREAMP_MAX_DB: i8 = 12;
/// Where auto-normalisation tries to put a track's gated RMS level, in dBFS,
/// and how far it is allowed to move a track to get there.
pub const NORMALIZE_TARGET_DB: f32 = -18.0;
pub const NORMALIZE_RANGE_DB: (f32, f32) = (-18.0, 12.0);
fn default_width() -> u8 {
    150
}
fn default_hiss() -> u8 {
    50
}
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct FxParams {
    pub mode: FxMode,
    pub scratch: u8,
    pub distortion: u8,
    pub age: u8,
    pub saturation: u8,
    pub wow: u8,
    pub flutter: u8,
    #[serde(default = "default_hiss")]
    pub hiss: u8,
    /// Gain in dB applied before everything else.
    #[serde(default)]
    pub preamp: i8,
    /// Add each track's measured normalisation gain to the preamp.
    #[serde(default)]
    pub auto_normalize: bool,
    /// Normalisation gain for the current track in dB, set by the app per
    /// track; it is state, not a setting, so it is never saved.
    #[serde(skip)]
    pub track_gain: f32,
    #[serde(default)]
    pub eq: EqParams,
    #[serde(default)]
    pub loudness: bool,
    #[serde(default)]
    pub stereo: StereoMode,
    #[serde(default = "default_width")]
    pub width: u8,
}
impl FxParams {
    /// User preamp plus the track's normalisation gain when that is on, in dB.
    pub fn effective_preamp_db(&self) -> f32 {
        let user = self.preamp.clamp(PREAMP_MIN_DB, PREAMP_MAX_DB) as f32;
        if self.auto_normalize {
            user + self
                .track_gain
                .clamp(NORMALIZE_RANGE_DB.0, NORMALIZE_RANGE_DB.1)
        } else {
            user
        }
    }
    pub(super) fn preamp_linear(&self) -> f32 {
        10f32.powf(self.effective_preamp_db() / 20.0)
    }
}
impl Default for FxParams {
    fn default() -> Self {
        Self {
            mode: FxMode::Off,
            scratch: 30,
            distortion: 15,
            age: 25,
            saturation: 35,
            wow: 20,
            flutter: 15,
            hiss: default_hiss(),
            preamp: 0,
            auto_normalize: false,
            track_gain: 0.0,
            eq: EqParams::default(),
            loudness: false,
            stereo: StereoMode::Normal,
            width: default_width(),
        }
    }
}
pub type SharedParams = Arc<Mutex<FxParams>>;
