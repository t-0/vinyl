#[derive(Clone, Debug)]
pub struct Album {
    pub artist: String,
    pub name: String,
    pub year: Option<u32>,
    pub start: usize,
    pub end: usize,
}
