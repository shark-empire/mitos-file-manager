use crate::filesystem::metadata::{self, FileMetadata};
use crate::mime::{detector, icons, thumbnail};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Item {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub metadata: FileMetadata,
    pub mime: String,
    pub icon_name: String,
    pub thumbnail_path: String,
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

            let mime = if is_dir {
                "inode/directory".to_string()
            } else {
                detector::guess_mime(&path)
            };

            let icon_name = icons::icon_name_for_mime(&mime, is_dir);

            let thumbnail_path = if is_dir {
                String::new()
            } else {
                thumbnail::thumbnail_path_for(&path, &mime, metadata.size)
            };

            items.push(Item {
                path,
                name,
                is_dir,
                metadata,
                mime,
                icon_name,
                thumbnail_path,
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
