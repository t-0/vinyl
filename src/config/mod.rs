mod editor_mode;
#[cfg(test)]
mod tests;
use anyhow::{Context, Result, anyhow};
pub use editor_mode::EditorMode;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
pub struct Config {
    pub music_home: PathBuf,
    pub index_file: PathBuf,
    /// Command that opens a track for editing (`audio.editor`), split into
    /// program and arguments; the file path is appended when it runs.
    pub audio_editor: Option<Vec<String>>,
    pub audio_editor_mode: EditorMode,
}
/// Splits a command line into words, honouring single and double quotes
/// and backslash escapes, so `audio.editor = "open -a \"Logic Pro\""` works.
fn split_command(raw: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') | (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    in_word = true;
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}
pub fn home_dir() -> Result<PathBuf> {
    std::env::home_dir().ok_or_else(|| anyhow!("could not determine home directory"))
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
const KNOWN_KEYS: [&str; 4] = [
    "music.home",
    "index.file",
    "audio.editor",
    "audio.editor.mode",
];
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur.push((prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}
/// A misspelt key would otherwise be silently ignored and the default used,
/// which is much harder to notice than an error at startup.
fn check_keys(map: &HashMap<String, String>, path: &Path) -> Result<()> {
    let mut unknown: Vec<&String> = map
        .keys()
        .filter(|k| !KNOWN_KEYS.contains(&k.as_str()))
        .collect();
    unknown.sort();
    let Some(key) = unknown.first() else {
        return Ok(());
    };
    let hint = KNOWN_KEYS
        .iter()
        .map(|k| (edit_distance(key, k), *k))
        .min()
        .filter(|(d, _)| *d <= 4)
        .map(|(_, k)| format!(" (did you mean `{k}`?)"))
        .unwrap_or_default();
    Err(anyhow!(
        "unknown setting `{key}` in {}{hint}; known settings: {}",
        path.display(),
        KNOWN_KEYS.join(", ")
    ))
}
pub fn load() -> Result<Config> {
    let path = config_path()?;
    if !path.exists() {
        let default = "# vinyl configuration\n\
# Folder containing your music (scanned recursively)\n\
music.home = ~/Music\n\
\n\
# Program that opens the selected track when you press E, e.g.\n\
#   audio.editor = /Applications/Audacity.app\n\
#   audio.editor = sox-editor --verbose\n\
#   audio.editor = myedit --input {file} --loop\n\
# {file} stands for the track path; without it the path goes last.\n\
#audio.editor =\n\
# gui: launch it and keep going. cli: the editor takes over this terminal\n\
# and vinyl waits for it to finish.\n\
#audio.editor.mode = gui\n";
        fs::write(&path, default)
            .with_context(|| format!("writing default config to {}", path.display()))?;
    }
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let map = parse_kv(&text);
    check_keys(&map, &path)?;
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
    let audio_editor = map
        .get("audio.editor")
        .map(|s| split_command(s))
        .filter(|words| !words.is_empty())
        .map(|mut words| {
            words[0] = expand_tilde(&words[0])
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| words[0].clone());
            words
        });
    let audio_editor_mode = map
        .get("audio.editor.mode")
        .map(|s| EditorMode::parse(s))
        .transpose()?
        .unwrap_or_default();
    Ok(Config {
        music_home,
        index_file,
        audio_editor,
        audio_editor_mode,
    })
}
impl Config {
    /// A track path for display: relative to `music.home` when it lives
    /// there, otherwise the same as [`display_path`].
    pub fn library_path(&self, p: &Path) -> String {
        match p.strip_prefix(&self.music_home) {
            Ok(rest) if !rest.as_os_str().is_empty() => rest.display().to_string(),
            _ => display_path(p),
        }
    }
}
pub fn display_path(p: &Path) -> String {
    if let Ok(home) = home_dir()
        && let Ok(rest) = p.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    p.display().to_string()
}
