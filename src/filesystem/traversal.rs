use mitos_utils::common::safewalk;

pub fn calculate_folder_size(path: &Path) -> u64 {
    let mut total_size = 0;
    // safewalk handles errors gracefully and prevents TOCTOU bugs
    for entry in safewalk(path) {
        if let Ok(entry) = entry {
            if entry.file_type().is_file() {
                if let Ok(metadata) = entry.metadata() {
                    total_size += metadata.len();
                }
            }
        }
    }
    total_size
}
