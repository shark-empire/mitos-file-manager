use crate::filesystem::directory;
use crate::search::filters::SearchFilters;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

/// A search stops collecting at this many hits. The results all live in
/// memory (and in the grid) at once, and nobody scrolls through more than
/// this -- if they hit the cap the query needs narrowing, not more rows.
pub const MAX_RESULTS: usize = 5000;

/// Files bigger than this are never opened by a content search.
const MAX_CONTENT_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// How much of a file is read at a time while looking for text in it.
const CONTENT_CHUNK_BYTES: usize = 64 * 1024;

pub struct SearchResult {
    pub item: directory::Item,
}

pub fn start_search(
    root: PathBuf,
    filters: SearchFilters,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<Vec<SearchResult>>,
) {
    std::thread::spawn(move || {
        let results = search_tree(&root, &filters, &cancel);

        // A cancelled search (the user started another, or cleared the box)
        // just goes quiet: dropping the sender is how the UI side learns
        // there's nothing to show.
        if !cancel.load(Ordering::Relaxed) {
            let _ = sender.send(results);
        }
    });
}

fn search_tree(root: &Path, filters: &SearchFilters, cancel: &AtomicBool) -> Vec<SearchResult> {
    let query_lower = filters.query.to_lowercase();
    // Content matching is ASCII-case-insensitive (the file is lowercased a
    // byte at a time as it streams by), so the needle is folded the same
    // way instead of with full Unicode lowercasing.
    let content_needle = filters.query.to_ascii_lowercase().into_bytes();
    let has_query = !filters.query.is_empty();
    // With neither box ticked there'd be no way for a query to match, so
    // fall back to the file name -- the search bar's default.
    let by_name = filters.match_file_name || !filters.match_content;

    let mut results: Vec<SearchResult> = Vec::new();
    // An explicit stack instead of recursion: a very deep tree can't
    // overflow the thread's stack.
    let mut pending: Vec<PathBuf> = vec![root.to_path_buf()];

    'walk: while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };

        for entry in entries.flatten() {
            if cancel.load(Ordering::Relaxed) || results.len() >= MAX_RESULTS {
                break 'walk;
            }

            let name = entry.file_name().to_string_lossy().to_string();

            if !filters.include_hidden && name.starts_with('.') {
                continue;
            }

            let path = entry.path();

            // `DirEntry::file_type` doesn't follow symlinks: a link to a
            // folder is listed as a result but never descended into, so a
            // link back up the tree can't send the search around in
            // circles.
            if filters.recursive && entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                pending.push(path.clone());
            }

            let name_hit = has_query && by_name && name.to_lowercase().contains(&query_lower);

            // Cheapest test first: a name that can't match and a search that
            // isn't looking inside files means there's no reason to build
            // the (much more expensive) full `Item` for this entry.
            if has_query && !name_hit && !filters.match_content {
                continue;
            }

            let item = directory::item_for_path(path, name);

            if !passes_type_and_size(&item, filters) {
                continue;
            }

            if has_query
                && !name_hit
                && (item.is_dir || !file_contains(&item.path, &content_needle, cancel))
            {
                continue;
            }

            results.push(SearchResult { item });
        }
    }

    // Folders first, then by name -- the same order a normal folder view
    // uses.
    results.sort_by(|a, b| match (a.item.is_dir, b.item.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a
            .item
            .name
            .to_lowercase()
            .cmp(&b.item.name.to_lowercase())
            .then_with(|| a.item.path.cmp(&b.item.path)),
    });

    results
}

fn passes_type_and_size(item: &directory::Item, filters: &SearchFilters) -> bool {
    if !filters.file_types.is_empty()
        && !filters
            .file_types
            .iter()
            .any(|file_type| file_type.matches_mime(&item.mime))
    {
        return false;
    }

    if let Some(min_size) = filters.min_size_bytes {
        if item.metadata.size < min_size {
            return false;
        }
    }

    if let Some(max_size) = filters.max_size_bytes {
        if item.metadata.size > max_size {
            return false;
        }
    }

    true
}

/// Does the file at `path` contain `needle` (already ASCII-lowercased)?
///
/// Reads in fixed-size chunks, keeping the last `needle.len() - 1` bytes of
/// each chunk so a match that straddles a chunk boundary is still found --
/// memory use is one chunk however big the file is. Files over
/// `MAX_CONTENT_FILE_BYTES`, and anything that looks binary (a NUL byte in
/// the first chunk), are skipped.
fn file_contains(path: &Path, needle: &[u8], cancel: &AtomicBool) -> bool {
    if needle.is_empty() {
        return false;
    }

    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };

    if !metadata.is_file() || metadata.len() > MAX_CONTENT_FILE_BYTES {
        return false;
    }

    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };

    let mut chunk = vec![0u8; CONTENT_CHUNK_BYTES];
    let mut carry: Vec<u8> = Vec::new();
    let mut first_chunk = true;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }

        let read = match file.read(&mut chunk) {
            Ok(0) | Err(_) => return false,
            Ok(read) => read,
        };

        if first_chunk {
            if chunk[..read].contains(&0) {
                return false;
            }

            first_chunk = false;
        }

        let mut window = std::mem::take(&mut carry);
        window.extend(chunk[..read].iter().map(|byte| byte.to_ascii_lowercase()));

        if window
            .windows(needle.len())
            .any(|candidate| candidate == needle)
        {
            return true;
        }

        let keep = (needle.len() - 1).min(window.len());
        carry = window.split_off(window.len() - keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::filters::FileTypeFilter;
    use crate::util::test_support::scratch_dir;

    fn names(results: &[SearchResult]) -> Vec<String> {
        let mut names: Vec<String> = results
            .iter()
            .map(|result| result.item.name.clone())
            .collect();

        names.sort();
        names
    }

    /// root/
    ///   notes.txt        "remember the MILK"
    ///   photo.png        (not really an image)
    ///   sub/
    ///     deep.txt       "more milk here"
    ///     .secret.txt    "milk in a hidden file"
    ///     loop -> root   (a symlink back up the tree)
    fn sample_tree(tag: &str) -> PathBuf {
        let root = scratch_dir(tag);

        fs::write(root.join("notes.txt"), "remember the MILK").unwrap();
        fs::write(root.join("photo.png"), "not really a png").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("deep.txt"), "more milk here").unwrap();
        fs::write(
            root.join("sub").join(".secret.txt"),
            "milk in a hidden file",
        )
        .unwrap();
        std::os::unix::fs::symlink(&root, root.join("sub").join("loop")).unwrap();

        root
    }

    #[test]
    fn finds_files_by_name_in_subfolders() {
        let root = sample_tree("search-name");
        let filters = SearchFilters {
            query: "DEEP".to_string(),
            ..SearchFilters::default()
        };

        let results = search_tree(&root, &filters, &AtomicBool::new(false));

        assert_eq!(names(&results), vec!["deep.txt"]);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn non_recursive_search_stays_in_the_top_folder() {
        let root = sample_tree("search-flat");
        let filters = SearchFilters {
            query: "txt".to_string(),
            recursive: false,
            ..SearchFilters::default()
        };

        let results = search_tree(&root, &filters, &AtomicBool::new(false));

        assert_eq!(names(&results), vec!["notes.txt"]);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn content_search_looks_inside_files_skips_hidden_and_survives_symlink_loops() {
        let root = sample_tree("search-content");
        let mut filters = SearchFilters {
            query: "milk".to_string(),
            match_content: true,
            ..SearchFilters::default()
        };

        // Returning at all proves the `loop` link wasn't followed.
        let results = search_tree(&root, &filters, &AtomicBool::new(false));
        assert_eq!(names(&results), vec!["deep.txt", "notes.txt"]);

        filters.include_hidden = true;
        let results = search_tree(&root, &filters, &AtomicBool::new(false));
        assert_eq!(
            names(&results),
            vec![".secret.txt", "deep.txt", "notes.txt"]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_type_filter_alone_lists_everything_of_that_type() {
        let root = sample_tree("search-type");
        let filters = SearchFilters {
            file_types: vec![FileTypeFilter::Folders],
            ..SearchFilters::default()
        };

        let results = search_tree(&root, &filters, &AtomicBool::new(false));

        // `sub`, plus the symlink to a folder (listed, never entered).
        assert_eq!(names(&results), vec!["loop", "sub"]);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_cancelled_search_returns_nothing() {
        let root = sample_tree("search-cancel");
        let filters = SearchFilters {
            query: "txt".to_string(),
            ..SearchFilters::default()
        };

        assert!(search_tree(&root, &filters, &AtomicBool::new(true)).is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_contains_is_ascii_case_insensitive_and_skips_binaries() {
        let dir = scratch_dir("file-contains");
        let cancel = AtomicBool::new(false);

        let plain = dir.join("plain.txt");
        fs::write(&plain, "Hello, Needle World").unwrap();
        assert!(file_contains(&plain, b"needle", &cancel));
        assert!(!file_contains(&plain, b"absent", &cancel));
        assert!(!file_contains(&plain, b"", &cancel));

        let binary = dir.join("blob.bin");
        fs::write(&binary, b"needle\0\0\0").unwrap();
        assert!(!file_contains(&binary, b"needle", &cancel));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_contains_finds_a_match_that_straddles_a_chunk_boundary() {
        let dir = scratch_dir("file-boundary");
        let cancel = AtomicBool::new(false);

        // "needle" begins 3 bytes before the end of the first chunk.
        let mut data = vec![b'a'; CONTENT_CHUNK_BYTES - 3];
        data.extend_from_slice(b"needle");
        data.extend_from_slice(&[b'b'; 100]);

        let path = dir.join("boundary.txt");
        fs::write(&path, &data).unwrap();

        assert!(file_contains(&path, b"needle", &cancel));
        assert!(!file_contains(&path, b"needlx", &cancel));

        let _ = fs::remove_dir_all(&dir);
    }
}
