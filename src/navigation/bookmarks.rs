use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub path: PathBuf,
}

pub fn config_dir() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("mitos");
    path.push("file-manager");
    path
}

pub fn bookmarks_file() -> PathBuf {
    config_dir().join("bookmarks.json")
}

pub fn load() -> Vec<Bookmark> {
    let path = bookmarks_file();
    if !path.exists() {
        return Vec::new();
    }

    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub fn save(bookmarks: &[Bookmark]) {
    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);

    let path = bookmarks_file();
    if let Ok(content) = serde_json::to_string_pretty(bookmarks) {
        let _ = fs::write(path, content);
    }
}

pub fn add(bookmarks: &mut Vec<Bookmark>, name: String, path: PathBuf) {
    if bookmarks.iter().any(|b| b.path == path) {
        return;
    }
    bookmarks.push(Bookmark { name, path });
    save(bookmarks);
}

pub fn remove(bookmarks: &mut Vec<Bookmark>, path: &Path) {
    let len_before = bookmarks.len();
    bookmarks.retain(|b| b.path != path);
    if bookmarks.len() != len_before {
        save(bookmarks);
    }
}
