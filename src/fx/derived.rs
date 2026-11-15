use super::FxMode;
#[derive(Clone, Copy, Default)]
pub(super) struct Derived {
    pub(super) mode: FxMode,
    /// Linear preamp gain; 1.0 is 0 dB.
    pub(super) preamp: f32,
    pub(super) drive: f32,
    pub(super) bias: f32,
    pub(super) makeup: f32,
    pub(super) hiss: f32,
    pub(super) wow_depth: f32,
    pub(super) flutter_depth: f32,
    pub(super) lfo_scale: f32,
    pub(super) pop_prob: f32,
    pub(super) pop_level: f32,
    pub(super) dropout_prob: f32,
    pub(super) cutoff: f32,
    pub(super) eq_bands: usize,
    pub(super) loudness: bool,
    pub(super) side_gain: Option<f32>,
}
