#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Artist,
    Track,
    Album,
    ArtistTrack,
}
impl SortKey {
    pub const ALL: [SortKey; 4] = [
        SortKey::Artist,
        SortKey::Track,
        SortKey::Album,
        SortKey::ArtistTrack,
    ];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Artist => "Artist",
            SortKey::Track => "Track",
            SortKey::Album => "Album",
            SortKey::ArtistTrack => "Artist/Track",
        }
    }
    pub fn describe(self) -> &'static str {
        match self {
            SortKey::Artist => "artist, then album, then track number",
            SortKey::Track => "track title, then artist",
            SortKey::Album => "album, then artist, then track number",
            SortKey::ArtistTrack => "artist, then track title, then album",
        }
    }
}
