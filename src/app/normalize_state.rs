#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NormalizeState {
    Off,
    /// On, but nothing is playing.
    Idle,
    Measuring,
    Gain(f32),
}
