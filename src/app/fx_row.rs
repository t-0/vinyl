#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FxRow {
    Mode,
    Scratch,
    Distortion,
    Age,
    Saturation,
    Wow,
    Flutter,
    Hiss,
}
impl FxRow {
    pub fn label(self) -> &'static str {
        match self {
            FxRow::Mode => "Mode",
            FxRow::Scratch => "Scratch",
            FxRow::Distortion => "Distortion",
            FxRow::Age => "Tape age",
            FxRow::Saturation => "Saturation",
            FxRow::Wow => "Wow",
            FxRow::Flutter => "Flutter",
            FxRow::Hiss => "Hiss",
        }
    }
}
