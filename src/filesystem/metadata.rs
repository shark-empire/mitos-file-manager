use std::fs;
use std::path::Path;
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct FileMetadata {
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub permissions: String,
    pub is_symlink: bool,
}

pub fn for_path(path: &Path) -> FileMetadata {
    let symlink_metadata = fs::symlink_metadata(path).ok();

    let metadata = fs::metadata(path)
        .ok()
        .or_else(|| fs::symlink_metadata(path).ok());

    let is_symlink = symlink_metadata
        .as_ref()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);

    let size = metadata.as_ref().map_or(0, |m| m.len());

    let modified = metadata.as_ref().and_then(|m| m.modified().ok());

    let permissions = metadata.as_ref().map_or_else(
        || "?".to_string(),
        |m| permission_string(&m.permissions()),
    );

    FileMetadata {
        size,
        modified,
        permissions,
        is_symlink,
    }
}

fn permission_string(permissions: &fs::Permissions) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = permissions.mode();

        let bits = [
            (0o400, 'r'),
            (0o200, 'w'),
            (0o100, 'x'),
            (0o040, 'r'),
            (0o020, 'w'),
            (0o010, 'x'),
            (0o004, 'r'),
            (0o002, 'w'),
            (0o001, 'x'),
        ];

        bits.into_iter()
            .map(|(bit, ch)| if mode & bit != 0 { ch } else { '-' })
            .collect()
    }

    #[cfg(not(unix))]
    {
        let _ = permissions;
        "?".to_string()
    }
}

pub fn format_size(size: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

    let mut value = size as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} B", size)
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

pub fn format_modified(modified: Option<SystemTime>) -> String {
    let Some(time) = modified else {
        return "-".to_string();
    };

    let Ok(elapsed) = SystemTime::now().duration_since(time) else {
        return "-".to_string();
    };

    let secs = elapsed.as_secs();

    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
