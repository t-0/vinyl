use super::*;
use std::fs;
fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vinyl-index-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
fn names(files: &[PathBuf]) -> Vec<String> {
    let mut v: Vec<String> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}
#[test]
fn walk_finds_audio_recursively_and_skips_hidden_entries() {
    let root = temp("walk");
    fs::create_dir_all(root.join("a/b")).unwrap();
    fs::create_dir_all(root.join(".hidden")).unwrap();
    for f in [
        "one.mp3",
        "a/two.flac",
        "a/b/three.m4a",
        "a/notes.txt",
        ".hidden/four.mp3",
        "a/.five.mp3",
    ] {
        fs::write(root.join(f), b"").unwrap();
    }
    let (mut files, mut unreadable) = (Vec::new(), Vec::new());
    walk(&root, &mut files, &mut unreadable);
    assert_eq!(names(&files), ["one.mp3", "three.m4a", "two.flac"]);
    assert!(unreadable.is_empty(), "{unreadable:?}");
}
#[cfg(unix)]
#[test]
fn walk_follows_symlinks_once_and_survives_loops() {
    use std::os::unix::fs::symlink;
    let root = temp("links");
    fs::create_dir_all(root.join("real")).unwrap();
    fs::write(root.join("real/song.mp3"), b"").unwrap();
    symlink(root.join("real"), root.join("alias")).unwrap();
    symlink(&root, root.join("real/loop")).unwrap();
    symlink(root.join("missing"), root.join("dangling")).unwrap();
    let (mut files, mut unreadable) = (Vec::new(), Vec::new());
    walk(&root, &mut files, &mut unreadable);
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(names(&files), ["song.mp3"]);
    assert_eq!(unreadable.len(), 1, "{unreadable:?}");
    assert!(unreadable[0].ends_with("dangling"));
}
