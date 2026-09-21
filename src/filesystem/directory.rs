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

/// Build the full `Item` -- metadata, MIME type, icon, thumbnail -- for one
/// path. Split out of `read_items` so the search engine can pay that cost
/// only for the entries that actually match, instead of for every file it
/// walks past.
pub fn item_for_path(path: PathBuf, name: String) -> Item {
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

    Item {
        path,
        name,
        is_dir,
        metadata,
        mime,
        icon_name,
        thumbnail_path,
    }
}

pub fn read_items(path: &Path, show_hidden: bool) -> Vec<Item> {
    let mut items = Vec::new();

    if let Ok(read_dir) = fs::read_dir(path) {
        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();

            if !show_hidden && name.starts_with('.') {
                continue;
            }

            items.push(item_for_path(entry.path(), name));
        }
    }

    items.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    items
}
