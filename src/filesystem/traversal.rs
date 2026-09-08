use mitos_utils::common::paths;
use std::path::Path;

pub fn calculate_folder_size(path: &Path) -> u64 {
    let mut total_size = 0;

    // Use paths::walk from mitos-utils which safely collects all paths in a tree
    if let Ok(entries) = paths::walk(path) {
        for entry in entries {
            if let Ok(metadata) = std::fs::symlink_metadata(&entry) {
                if metadata.is_file() {
                    total_size += metadata.len();
                }
            }
        }
    }
    total_size
}
