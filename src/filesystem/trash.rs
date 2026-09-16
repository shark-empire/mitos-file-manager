use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_unix;

#[derive(Clone)]
pub struct TrashItem {
    pub trash_name: String,
    pub original_path: PathBuf,
    pub file_path: PathBuf,
    pub info_path: PathBuf,
    /// Where this item's trash can lives -- "Home" for the ordinary
    /// `$XDG_DATA_HOME/Trash`, or the volume's display name for a
    /// per-device trash can found on another mounted filesystem (see
    /// `list` below).
    pub location_label: String,
    /// Raw `DeletionDate=` value from the `.trashinfo` file, reformatted
    /// as `"YYYY-MM-DD hh:mm"` if it parsed, for display.
    pub deletion_date: Option<String>,
}

/// All trashed items MITOS Files can find: the home trash, plus (per the
/// XDG trash spec) any `.Trash-$uid` / `.Trash/$uid` directory on the
/// volumes in `extra_roots` -- pass mounted volumes as (display name,
/// mount path) pairs, e.g. from `sidebar::external_mounts()`, so a file
/// trashed from a USB drive or network share shows up here too, not just
/// ones under your home directory.
///
/// `operations::trash::delete` (backed by the `trash` crate) already
/// follows this same spec when *writing* a file to trash -- a file
/// deleted from another filesystem already ends up in the right
/// per-device trash can, not copied all the way to the home one. This is
/// what makes `list`/`restore`/`empty` able to find what it wrote.
pub fn list(extra_roots: &[(String, PathBuf)]) -> Vec<TrashItem> {
    let mut items = Vec::new();

    if let Some(root) = trash_root() {
        items.extend(list_at(&root, "Home"));
    }

    for (label, topdir) in extra_roots {
        for candidate in topdir_trash_candidates(topdir) {
            items.extend(list_at(&candidate, label));
        }
    }

    items.sort_by(|a, b| {
        a.location_label
            .cmp(&b.location_label)
            .then_with(|| a.trash_name.cmp(&b.trash_name))
    });

    items
}

fn list_at(root: &Path, location_label: &str) -> Vec<TrashItem> {
    let info_dir = root.join("info");
    let files_dir = root.join("files");

    let Ok(entries) = fs::read_dir(info_dir) else {
        return Vec::new();
    };

    let mut items = Vec::new();

    for entry in entries.flatten() {
        let info_path = entry.path();

        let is_trashinfo = info_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext == "trashinfo")
            .unwrap_or(false);

        if !is_trashinfo {
            continue;
        }

        let Some(trash_name) = info_path
            .file_stem()
            .map(|name| name.to_string_lossy().to_string())
        else {
            continue;
        };

        let file_path = files_dir.join(&trash_name);

        if !file_path.exists() {
            continue;
        }

        // Read the .trashinfo file once and pull both fields out of it,
        // rather than opening it twice.
        let info_content = fs::read_to_string(&info_path).ok();
        let original_path = info_content
            .as_deref()
            .and_then(parse_original_path)
            .unwrap_or_else(|| file_path.clone());
        let deletion_date = info_content.as_deref().and_then(parse_deletion_date);

        items.push(TrashItem {
            trash_name,
            original_path,
            file_path,
            info_path,
            location_label: location_label.to_string(),
            deletion_date,
        });
    }

    items
}

pub fn restore(item: &TrashItem) -> io::Result<()> {
    if item.original_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "The original file already exists",
        ));
    }

    if let Some(parent) = item.original_path.parent() {
        fs::create_dir_all(parent)?;
    }

    move_with_fallback(&item.file_path, &item.original_path)?;

    if item.info_path.exists() {
        fs::remove_file(&item.info_path)?;
    }

    Ok(())
}

/// Permanently delete a single trashed item, without touching anything
/// else in whichever trash can it's in.
pub fn delete_forever(item: &TrashItem) -> io::Result<()> {
    remove_any(&item.file_path)?;

    if item.info_path.exists() {
        fs::remove_file(&item.info_path)?;
    }

    Ok(())
}

/// Empty the home trash and every reachable trash can under `extra_roots`.
pub fn empty(extra_roots: &[(String, PathBuf)]) -> io::Result<()> {
    if let Some(root) = trash_root() {
        empty_at(&root)?;
    }

    for (_, topdir) in extra_roots {
        for candidate in topdir_trash_candidates(topdir) {
            empty_at(&candidate)?;
        }
    }

    Ok(())
}

fn empty_at(root: &Path) -> io::Result<()> {
    let files_dir = root.join("files");
    let info_dir = root.join("info");

    if files_dir.exists() {
        for entry in fs::read_dir(&files_dir)? {
            let entry = entry?;
            remove_any(&entry.path())?;
        }
    }

    if info_dir.exists() {
        for entry in fs::read_dir(&info_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                fs::remove_file(path)?;
            }
        }
    }

    Ok(())
}

fn trash_root() -> Option<PathBuf> {
    let data_home = data_home()?;
    Some(data_home.join("Trash"))
}

fn data_home() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(value);

        if path.is_absolute() {
            return Some(path);
        }
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(home.join(".local/share"))
}

/// The two spec-defined locations a non-home trash can might be at for a
/// given mount point (`topdir`) -- both are returned regardless of which
/// (if either) currently has content, since `list_at`/`empty_at` are
/// no-ops on one that doesn't exist, and checking both means anything
/// trashed there under either scheme gets found.
fn topdir_trash_candidates(topdir: &Path) -> Vec<PathBuf> {
    let uid = unsafe { libc::getuid() };
    let mut candidates = Vec::new();

    // $topdir/.Trash/$uid -- only valid if $topdir/.Trash exists, isn't a
    // symlink, and has the sticky bit set. The spec requires the sticky-
    // bit check so a pre-existing, world-writable .Trash can't be used to
    // intercept another user's files.
    let shared = topdir.join(".Trash");

    if is_valid_shared_trash_dir(&shared) {
        candidates.push(shared.join(uid.to_string()));
    }

    // $topdir/.Trash-$uid -- doesn't need the sticky-bit dance since the
    // uid is already baked into the directory name.
    candidates.push(topdir.join(format!(".Trash-{uid}")));

    candidates
}

fn is_valid_shared_trash_dir(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };

    if !meta.is_dir() || meta.file_type().is_symlink() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o1000 != 0
    }

    #[cfg(not(unix))]
    {
        false
    }
}

fn parse_original_path(content: &str) -> Option<PathBuf> {
    for line in content.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("Path=") {
            let rest = rest.trim();
            let rest = rest.strip_prefix("file://").unwrap_or(rest);

            return Some(PathBuf::from(percent_decode(rest)));
        }
    }

    None
}

/// `DeletionDate=YYYY-MM-DDThh:mm:ss` (the XDG trash spec's format, local
/// time, no timezone marker) -> `"YYYY-MM-DD hh:mm"` for display. Falls
/// back to the raw value if it's not in that exact shape, rather than
/// hiding it -- still more useful than nothing.
fn parse_deletion_date(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();

        if let Some(rest) = line.strip_prefix("DeletionDate=") {
            let rest = rest.trim();

            return Some(match rest.split_once('T') {
                Some((date, time)) => {
                    let time = time.get(0..5).unwrap_or(time);
                    format!("{date} {time}")
                }
                None => rest.to_string(),
            });
        }
    }

    None
}

fn percent_decode(input: &str) -> String {
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

fn move_with_fallback(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(_) => Ok(()),
        Err(err) => {
            const EXDEV: i32 = 18;

            if err.raw_os_error() == Some(EXDEV) {
                copy_any(source, destination)?;
                remove_any(source)?;
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

fn copy_any(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;

    if metadata.is_dir() {
        copy_dir(source, destination)
    } else {
        fs::copy(source, destination)?;
        Ok(())
    }
}

fn copy_dir(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(entry.path())?;

            #[cfg(unix)]
            symlink_unix(link_target, &target)?;

            #[cfg(not(unix))]
            {
                let _ = link_target;
            }
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }

    Ok(())
}

fn remove_any(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}
