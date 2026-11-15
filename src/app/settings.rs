use crate::config::Config;
use crate::fx::FxParams;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
/// Everything the user sets while the app runs that should survive a
/// restart. The FX parameters are flattened so the file keeps the layout
/// older versions wrote (FX fields at the top level); fields added later
/// default when missing.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(flatten)]
    pub fx: FxParams,
    #[serde(default)]
    pub shuffle: bool,
    /// Playback speed as a percentage offset from normal (−50 to +100).
    #[serde(default)]
    pub speed_pct: i32,
    /// Path of the track that was playing, restored (paused) on the next start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_track: Option<PathBuf>,
}
pub(super) fn settings_file(config: &Config) -> std::path::PathBuf {
    config.index_file.with_file_name(".vinyl-fx.json")
}
pub fn load_settings(config: &Config) -> Settings {
    std::fs::read_to_string(settings_file(config))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
pub fn load_fx(config: &Config) -> FxParams {
    load_settings(config).fx
}
pub(super) fn write_settings(config: &Config, settings: &Settings) -> Result<()> {
    let json = serde_json::to_string_pretty(settings)?;
    std::fs::write(settings_file(config), json)?;
    Ok(())
}
