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
