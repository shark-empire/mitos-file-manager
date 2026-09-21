use gtk::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

/// Read the XDG recently-used database and return existing
/// file paths, newest first.
pub fn recent_files(limit: usize) -> Vec<PathBuf> {
    let Some(xbel) = xbel_path() else {
        return Vec::new();
    };

    let Ok(content) = fs::read_to_string(&xbel) else {
        return Vec::new();
    };

    let mut paths: Vec<PathBuf> = Vec::new();

    for line in content.lines() {
        let Some(href) = extract_href(line) else {
            continue;
        };

        let Some(local) = href.strip_prefix("file://") else {
            continue;
        };

        let path = PathBuf::from(crate::util::percent_decode(local));

        if path.exists() && !paths.contains(&path) {
            paths.push(path);
        }
    }

    // xbel appends newest entries last → reverse for newest-first.
    paths.reverse();
    paths.truncate(limit);
    paths
}

fn extract_href(line: &str) -> Option<String> {
    let start = line.find("href=\"")? + 6;
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Tell the desktop's recently-used list that `path` was just opened, so it
/// appears in this app's sidebar "Recent" section (and in other
/// applications' "recent files" menus). Best effort: the list is written
/// out later, from the main loop.
pub fn record(path: &Path) {
    let uri = gtk::gio::File::for_path(path).uri();

    gtk::RecentManager::default().add_item(&uri);
}

/// Where the desktop's recently-used list lives (also watched, cheaply, to
/// know when the sidebar's "Recent" section is out of date).
pub fn xbel_path() -> Option<PathBuf> {
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        if !data.is_empty() {
            return Some(PathBuf::from(data).join("recently-used.xbel"));
        }
    }

    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".local/share/recently-used.xbel"))
}
