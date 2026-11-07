use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
pub struct Config {
    pub music_home: PathBuf,
    pub index_file: PathBuf,
}
pub fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))
}
pub fn config_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("VINYL_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    Ok(home_dir()?.join(".vinyl"))
}
/// Where stderr goes while the UI is on screen (see `stderr.rs`).
pub fn log_path() -> PathBuf {
    config_path()
        .map(|p| p.with_extension("log"))
        .unwrap_or_else(|_| PathBuf::from("vinyl.log"))
}
fn expand_tilde(raw: &str) -> Result<PathBuf> {
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return Ok(home_dir()?.join(rest));
    }
    Ok(PathBuf::from(raw))
}
fn parse_kv(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let mut v = v.trim();
        if v.len() >= 2 {
            let b = v.as_bytes();
            if (b[0] == b'"' && b[b.len() - 1] == b'"')
                || (b[0] == b'\'' && b[b.len() - 1] == b'\'')
            {
                v = &v[1..v.len() - 1];
            }
        }
        map.insert(k.trim().to_string(), v.to_string());
    }
    map
}
pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        let default = "# vinyl configuration\n# Folder containing your music (scanned recursively)\nmusic.home = ~/Music\n";
        fs::write(&path, default)
            .with_context(|| format!("writing default config to {}", path.display()))?;
    }
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let map = parse_kv(&text);
    let music_home = map
        .get("music.home")
        .map(|s| expand_tilde(s))
        .transpose()?
        .unwrap_or(home_dir()?.join("Music"));
    if !music_home.is_dir() {
        return Err(anyhow!(
            "music.home `{}` (from {}) is not a directory",
            music_home.display(),
            path.display()
        ));
    }
    let index_file = match map.get("index.file") {
        Some(s) => expand_tilde(s)?,
        None => home_dir()?.join(".vinyl-index.json"),
    };
    Ok(Config {
        music_home,
        index_file,
    })
}
pub fn display_path(p: &Path) -> String {
    if let Ok(home) = home_dir()
        && let Ok(rest) = p.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}
