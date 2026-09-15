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

    let permissions = metadata
        .as_ref()
        .map_or_else(|| "?".to_string(), |m| permission_string(&m.permissions()));

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

/// Free space available to the current (unprivileged) user on the
/// filesystem that contains `path`, formatted the same way file sizes are.
///
/// `path` can be any file or directory on the target filesystem -- POSIX
/// `statvfs` reports stats for the whole containing filesystem, not just
/// that one entry, so the current directory being displayed is exactly
/// the right thing to pass in.
pub fn free_space_string(path: &Path) -> String {
    match free_space_bytes(path) {
        Some(bytes) => format_size(bytes),
        None => String::from("Unknown"),
    }
}

fn free_space_bytes(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();

    // SAFETY: `c_path` is a valid, NUL-terminated C string that outlives
    // this call, and `stat.as_mut_ptr()` points at a valid, correctly
    // sized (if uninitialized) `libc::statvfs` for `statvfs(3)` to write
    // into. We only read `stat` below after checking the call returned 0,
    // which per `statvfs(3)` means every field was filled in.
    let ok = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 };

    if !ok {
        return None;
    }

    // SAFETY: see above -- a 0 return guarantees `stat` is fully initialized.
    let stat = unsafe { stat.assume_init() };

    // `f_frsize` is the fragment size actually used to express block
    // counts; some filesystems report a different `f_bsize`, so prefer
    // `f_frsize` per statvfs(3). `f_bavail` (not `f_bfree`) is blocks
    // available to this (unprivileged) user, excluding any root-reserved
    // margin -- the same number `df` shows by default.
    let block_size = if stat.f_frsize > 0 {
        stat.f_frsize
    } else {
        stat.f_bsize
    };

    Some((stat.f_bavail as u64).saturating_mul(block_size as u64))
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
