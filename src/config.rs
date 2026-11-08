use anyhow::{Context, Result, anyhow};
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
    /// Which front end starts when neither `--gui` nor `--no-gui` is given.
    pub ui: Ui,
}
/// The front end (`ui`): the terminal UI or a desktop window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ui {
    #[default]
    Tui,
    Gui,
}
impl Ui {
    fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "tui" | "terminal" | "cli" => Ok(Self::Tui),
            "gui" | "window" | "desktop" => Ok(Self::Gui),
            other => Err(anyhow!("ui must be `tui` or `gui`, not `{other}`")),
        }
    }
}
/// How `audio.editor` behaves once launched (`audio.editor.mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EditorMode {
    /// The editor is a desktop app: start it and carry on.
    #[default]
    Gui,
    /// The editor runs in this terminal: hand the screen over and wait for it
    /// to finish before the UI comes back.
    Cli,
}
impl EditorMode {
    fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "gui" => Ok(Self::Gui),
            "cli" | "tui" | "terminal" => Ok(Self::Cli),
            other => Err(anyhow!(
                "audio.editor.mode must be `gui` or `cli`, not `{other}`"
            )),
        }
    }
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
const KNOWN_KEYS: [&str; 5] = [
    "music.home",
    "index.file",
    "audio.editor",
    "audio.editor.mode",
    "ui",
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
#audio.editor.mode = gui\n\
\n\
# tui: run in the terminal (default). gui: open a desktop window.\n\
# --gui and --no-gui on the command line override this.\n\
#ui = tui\n";
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
    let ui = map
        .get("ui")
        .map(|s| Ui::parse(s))
        .transpose()?
        .unwrap_or_default();
    Ok(Config {
        music_home,
        index_file,
        audio_editor,
        audio_editor_mode,
        ui,
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splits_command_words_and_quotes() {
        assert_eq!(split_command("audacity"), vec!["audacity"]);
        assert_eq!(
            split_command("  sox   --flag  x "),
            vec!["sox", "--flag", "x"]
        );
        assert_eq!(
            split_command(r#"open -a "Logic Pro" 'single q' esc\ aped"#),
            vec!["open", "-a", "Logic Pro", "single q", "esc aped"]
        );
        assert_eq!(split_command(r#"a "" b"#), vec!["a", "", "b"]);
        assert!(split_command("   ").is_empty());
    }
    #[test]
    fn rejects_unknown_keys_with_a_hint() {
        let p = Path::new("~/.vinyl");
        assert!(check_keys(&parse_kv("music.home = ~\naudio.editor = x\n"), p).is_ok());
        let err = check_keys(&parse_kv("auto.editor.mode = cli\n"), p)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown setting `auto.editor.mode`"), "{err}");
        assert!(err.contains("did you mean `audio.editor.mode`"), "{err}");
        let err = check_keys(&parse_kv("zzzzzzzzzzzz = 1\n"), p)
            .unwrap_err()
            .to_string();
        assert!(!err.contains("did you mean"), "{err}");
    }
    #[test]
    fn library_path_strips_music_home() {
        let c = Config {
            music_home: PathBuf::from("/m/lib"),
            index_file: PathBuf::from("/m/i.json"),
            audio_editor: None,
            audio_editor_mode: EditorMode::Gui,
            ui: Ui::default(),
        };
        assert_eq!(
            c.library_path(Path::new("/m/lib/A/B/01 t.mp3")),
            "A/B/01 t.mp3"
        );
        assert_eq!(c.library_path(Path::new("/other/x.mp3")), "/other/x.mp3");
        assert_eq!(c.library_path(Path::new("/m/lib")), "/m/lib");
    }
    #[test]
    fn parses_editor_mode() {
        assert_eq!(EditorMode::parse("gui").unwrap(), EditorMode::Gui);
        assert_eq!(EditorMode::parse(" CLI ").unwrap(), EditorMode::Cli);
        assert!(EditorMode::parse("web").is_err());
    }
    #[test]
    fn config_values_strip_quotes() {
        let m = parse_kv("audio.editor = \"open -a Audacity\"\naudio.editor.mode='cli'\n# c\n");
        assert_eq!(m["audio.editor"], "open -a Audacity");
        assert_eq!(m["audio.editor.mode"], "cli");
    }
}
