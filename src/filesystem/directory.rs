use crate::filesystem::metadata::{self, FileMetadata};
use crate::mime::{detector, icons};
use std::collections::HashMap;
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

/// Build the `Item` for one path -- metadata, MIME type, icon. (Thumbnails
/// are deliberately *not* looked up here: see `ui::grid_view`, which does it
/// for the rows actually on screen.) Used for search hits, which are few;
/// `read_items` uses the cheaper per-entry route below.
pub fn item_for_path(path: PathBuf, name: String) -> Item {
    let is_dir = path.is_dir();

    let metadata = metadata::for_path(&path);

    let mime = if is_dir {
        "inode/directory".to_string()
    } else {
        detector::guess_mime_by_name(&path, &name)
    };

    let icon_name = icons::icon_name_for_mime(&mime, is_dir);

    Item {
        path,
        name,
        is_dir,
        metadata,
        mime,
        icon_name,
        thumbnail_path: String::new(),
    }
}

/// The `Item` for one directory entry, using as little as possible:
///
/// * one `lstat` (`DirEntry::metadata`), plus a second only for a symlink,
///   instead of five or six `stat`-family calls per file;
/// * the MIME type from the file *name*, sniffing inside the file only when
///   the name doesn't say (see `detector::guess_mime_by_name`);
/// * the icon looked up once per distinct MIME type, not once per file --
///   `icons_by_mime` carries that memo across the whole listing.
fn item_from_entry(
    entry: &fs::DirEntry,
    name: String,
    icons_by_mime: &mut HashMap<String, String>,
) -> Item {
    let path = entry.path();
    let file_type = entry.file_type().ok();

    let is_symlink = file_type.map_or(false, |file_type| file_type.is_symlink());

    let link_metadata = entry.metadata().ok();
    let target_metadata = if is_symlink {
        fs::metadata(&path).ok()
    } else {
        None
    };

    // A link to a folder is a folder, as far as browsing goes.
    let is_dir = if is_symlink {
        target_metadata
            .as_ref()
            .map_or(false, |metadata| metadata.is_dir())
    } else {
        file_type.map_or(false, |file_type| file_type.is_dir())
    };

    let metadata = match &link_metadata {
        Some(link) => metadata::from_metadata(link, target_metadata.as_ref()),
        // Couldn't even `lstat` it (it vanished, or no permission): take
        // the slow path, which copes with that.
        None => metadata::for_path(&path),
    };

    let mime = if is_dir {
        "inode/directory".to_string()
    } else {
        detector::guess_mime_by_name(&path, &name)
    };

    let icon_name = icons_by_mime
        .entry(mime.clone())
        .or_insert_with(|| icons::icon_name_for_mime(&mime, is_dir))
        .clone();

    Item {
        path,
        name,
        is_dir,
        metadata,
        mime,
        icon_name,
        thumbnail_path: String::new(),
    }
}

/// Everything in `path`, folders first and then by name (case-insensitive).
///
/// This is the slow, blocking part of showing a folder, so callers run it
/// on a worker thread (`load_directory` in `main.rs`); nothing in here
/// touches GTK.
pub fn read_items(path: &Path, show_hidden: bool) -> Vec<Item> {
    let Ok(read_dir) = fs::read_dir(path) else {
        return Vec::new();
    };

    let mut icons_by_mime: HashMap<String, String> = HashMap::new();
    let mut items = Vec::new();

    for entry in read_dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        if !show_hidden && name.starts_with('.') {
            continue;
        }

        items.push(item_from_entry(&entry, name, &mut icons_by_mime));
    }

    // `sort_by_cached_key` lowercases each name once; sorting with
    // `to_lowercase()` inside the comparator would allocate on every one of
    // the n log n comparisons.
    items.sort_by_cached_key(|item| (!item.is_dir, item.name.to_lowercase()));

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn lists_folders_first_then_names_case_insensitively() {
        let dir = scratch_dir("listing-order");
        fs::write(dir.join("banana.txt"), "b").unwrap();
        fs::write(dir.join("Apple.txt"), "a").unwrap();
        fs::create_dir(dir.join("zebra")).unwrap();
        fs::create_dir(dir.join("Aardvark")).unwrap();

        let names: Vec<String> = read_items(&dir, true)
            .into_iter()
            .map(|item| item.name)
            .collect();

        assert_eq!(names, vec!["Aardvark", "zebra", "Apple.txt", "banana.txt"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hidden_files_are_left_out_unless_asked_for() {
        let dir = scratch_dir("listing-hidden");
        fs::write(dir.join(".secret"), "s").unwrap();
        fs::write(dir.join("visible"), "v").unwrap();

        assert_eq!(read_items(&dir, false).len(), 1);
        assert_eq!(read_items(&dir, true).len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn entries_carry_size_mime_and_the_symlink_flag() {
        let dir = scratch_dir("listing-details");
        fs::write(dir.join("notes.txt"), "twelve bytes").unwrap();
        fs::create_dir(dir.join("folder")).unwrap();
        std::os::unix::fs::symlink(dir.join("folder"), dir.join("to-folder")).unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), dir.join("dangling")).unwrap();

        let items = read_items(&dir, true);
        let find = |name: &str| items.iter().find(|item| item.name == name).unwrap();

        let notes = find("notes.txt");
        assert_eq!(notes.metadata.size, 12);
        assert!(notes.mime.starts_with("text/"));
        assert!(!notes.is_dir && !notes.metadata.is_symlink);
        assert!(notes.thumbnail_path.is_empty());

        assert!(find("folder").is_dir);
        assert_eq!(find("folder").mime, "inode/directory");

        // A link to a folder browses like a folder but is still flagged.
        assert!(find("to-folder").is_dir);
        assert!(find("to-folder").metadata.is_symlink);

        // A dangling link is listed, not dropped or treated as a folder.
        assert!(!find("dangling").is_dir);
        assert!(find("dangling").metadata.is_symlink);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_or_missing_folder_lists_as_empty() {
        assert!(read_items(Path::new("/no/such/folder"), true).is_empty());
    }

    #[test]
    fn files_of_one_type_share_an_icon() {
        let dir = scratch_dir("listing-icons");
        fs::write(dir.join("a.txt"), "a").unwrap();
        fs::write(dir.join("b.txt"), "b").unwrap();

        let items = read_items(&dir, true);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].icon_name, items[1].icon_name);
        assert!(!items[0].icon_name.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
