use std::fs;
use std::path::PathBuf;

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

        let path = PathBuf::from(url_decode(local));

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

fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }

        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).to_string()
}

fn xbel_path() -> Option<PathBuf> {
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        if !data.is_empty() {
            return Some(PathBuf::from(data).join("recently-used.xbel"));
        }
    }

    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".local/share/recently-used.xbel"))
}
