#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Songs,
    Albums,
    Artists,
}
impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Songs => "Songs",
            Section::Albums => "Albums",
            Section::Artists => "Artists",
        }
    }
}
