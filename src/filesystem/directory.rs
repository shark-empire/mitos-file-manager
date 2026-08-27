use crate::filesystem::metadata::{self, FileMetadata};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub metadata: FileMetadata,
}

pub fn read_items(path: &Path, show_hidden: bool) -> Vec<Item> {
    let mut items = Vec::new();

    if let Ok(read_dir) = fs::read_dir(path) {
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();

            if !show_hidden && name.starts_with('.') {
                continue;
            }

            let path = entry.path();
            let is_dir = path.is_dir();
            let metadata = metadata::for_path(&path);

            items.push(Item {
                path,
                name,
                is_dir,
                metadata,
            });
        }
    }

    items.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    items
}
