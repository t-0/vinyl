#[derive(Clone, Debug)]
pub struct Artist {
    pub name: String,
    pub album_start: usize,
    pub album_end: usize,
    pub track_start: usize,
    pub track_end: usize,
}
