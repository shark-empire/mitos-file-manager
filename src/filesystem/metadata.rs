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

/// Fail early -- before writing anything -- if `needed` bytes clearly won't
/// fit where `destination` lives, with a message saying how much is needed
/// and how much is there. A filesystem that reports zero available is
/// treated as "unknown", not "full": some network mounts do.
///
/// `destination` need not exist yet; its nearest existing ancestor is asked.
pub fn ensure_free_space(destination: &Path, needed: u64) -> Result<(), String> {
    let mut probe = destination;

    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return Ok(()),
        }
    }

    match free_space_bytes(probe) {
        Some(free) if free > 0 && needed > free => Err(format!(
            "Not enough free space: {} needed, {} available",
            format_size(needed),
            format_size(free)
        )),
        _ => Ok(()),
    }
}

/// Build a `FileMetadata` from metadata already in hand -- what a directory
/// listing gets for free from `DirEntry::metadata` -- instead of `for_path`'s
/// two or three separate `stat` calls per file.
///
/// `link_metadata` is the entry itself (an `lstat`); `target_metadata` is
/// what a symlink points at (`None` for anything that isn't a symlink, or a
/// dangling one). As in `for_path`, a link is described by its target's size,
/// time and permissions, falling back to the link's own if it dangles.
pub fn from_metadata(
    link_metadata: &fs::Metadata,
    target_metadata: Option<&fs::Metadata>,
) -> FileMetadata {
    let is_symlink = link_metadata.file_type().is_symlink();

    let shown = if is_symlink {
        target_metadata.unwrap_or(link_metadata)
    } else {
        link_metadata
    };

    FileMetadata {
        size: shown.len(),
        modified: shown.modified().ok(),
        permissions: permission_string(&shown.permissions()),
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

pub fn free_space_bytes(path: &Path) -> Option<u64> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn metadata_from_a_listing_matches_the_stat_based_version() {
        let dir = scratch_dir("metadata-from");
        let file = dir.join("data.bin");
        fs::write(&file, vec![0u8; 321]).unwrap();

        let link_metadata = fs::symlink_metadata(&file).unwrap();
        let fast = from_metadata(&link_metadata, None);
        let slow = for_path(&file);

        assert_eq!(fast.size, 321);
        assert_eq!(fast.size, slow.size);
        assert_eq!(fast.permissions, slow.permissions);
        assert!(!fast.is_symlink);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn free_space_check_rejects_the_impossible_and_allows_the_trivial() {
        let dir = scratch_dir("metadata-space");

        let refusal = ensure_free_space(&dir, u64::MAX).unwrap_err();
        assert!(refusal.starts_with("Not enough free space"), "{refusal}");

        // Nothing needed, and a destination that doesn't exist yet.
        assert!(ensure_free_space(&dir.join("not/yet/created"), 0).is_ok());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlink_reports_its_targets_size_but_is_flagged_as_a_link() {
        let dir = scratch_dir("metadata-link");
        let file = dir.join("real.bin");
        fs::write(&file, vec![0u8; 500]).unwrap();
        let link = dir.join("alias");
        std::os::unix::fs::symlink(&file, &link).unwrap();

        let link_metadata = fs::symlink_metadata(&link).unwrap();
        let target_metadata = fs::metadata(&link).unwrap();
        let described = from_metadata(&link_metadata, Some(&target_metadata));

        assert!(described.is_symlink);
        assert_eq!(described.size, 500);

        // Dangling: falls back to the link's own (tiny) size.
        fs::remove_file(&file).unwrap();
        let dangling = from_metadata(&fs::symlink_metadata(&link).unwrap(), None);
        assert!(dangling.is_symlink);
        assert!(dangling.size < 500);

        let _ = fs::remove_dir_all(&dir);
    }
}
