use crate::filesystem::directory;
use crate::search::filters::SearchFilters;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;

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
        let mut results = Vec::new();

        search_directory(&root, &filters, &cancel, &mut results);

        if !cancel.load(Ordering::Relaxed) {
            let _ = sender.send(results);
        }
    });
}

fn search_directory(
    dir: &Path,
    filters: &SearchFilters,
    cancel: &AtomicBool,
    results: &mut Vec<SearchResult>,
) {
    if cancel.load(Ordering::Relaxed) {
        return;
    }

    let items = directory::read_items(dir, true);

    for item in &items {
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        if matches_filters(item, filters) {
            results.push(SearchResult { item: item.clone() });
        }

        // Recurse into subdirectories
        if filters.recursive && item.is_dir {
            search_directory(&item.path, filters, cancel, results);
        }
    }
}

fn matches_filters(item: &directory::Item, filters: &SearchFilters) -> bool {
    // Query match
    if !filters.query.is_empty() {
        let query_lower = filters.query.to_lowercase();
        let name_lower = item.name.to_lowercase();

        if filters.match_file_name && !name_lower.contains(&query_lower) {
            return false;
        }
    }

    // File type filter
    if !filters.file_types.is_empty() {
        let matches_any = filters
            .file_types
            .iter()
            .any(|ft| ft.matches_mime(&item.mime));

        if !matches_any {
            return false;
        }
    }

    // Size filters
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
