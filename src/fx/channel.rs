use super::Biquad;
pub(super) struct Channel {
    pub(super) lowpass: Biquad,
    pub(super) delay: Vec<f32>,
    pub(super) wpos: usize,
    pub(super) eq: [Biquad; 7],
    pub(super) loud: [Biquad; 2],
}
