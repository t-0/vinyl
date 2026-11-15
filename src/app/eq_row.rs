#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EqRow {
    Preamp,
    Normalize,
    Mode,
    Band(usize),
    Loudness,
    Stereo,
    Width,
}
