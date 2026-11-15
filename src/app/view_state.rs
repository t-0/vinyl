/// What the three list cursors were on, by identity rather than index, so
/// they can be put back after the track list is rebuilt.
pub(super) struct ViewState {
    pub(super) song: Option<std::path::PathBuf>,
    pub(super) album: Option<(String, String)>,
    pub(super) artist: Option<String>,
    pub(super) offsets: [usize; 3],
}
