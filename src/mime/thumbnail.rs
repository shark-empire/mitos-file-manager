use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::path::{Path, PathBuf};

pub fn thumbnail_path_for(path: &Path, mime: &str, size: u64) -> String {
    if !crate::config::settings::thumbnails_enabled() {
        return String::new();
    }

    if let Some(existing) = freedesktop_thumbnail_path(path) {
        return existing.to_string_lossy().to_string();
    }

    let max_size = crate::config::settings::thumbnail_max_bytes();

    if size > 0 && size <= max_size && mime.starts_with("image/") {
        return path.to_string_lossy().to_string();
    }

    String::new()
}

fn freedesktop_thumbnail_path(path: &Path) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;

    let file = gio::File::for_path(path);
    let uri = file.uri().to_string();

    let hash = glib::Checksum::compute_checksum_for_string(glib::ChecksumType::Md5, &uri)?;

    let cache = home.join(".cache/thumbnails");

    for directory in ["large", "normal"] {
        let candidate = cache.join(directory).join(format!("{hash}.png"));

        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}
