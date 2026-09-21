//! Protected system paths.
//!
//! A file manager that will happily "Move to Trash" `/usr` (or `~`, or a
//! USB stick's mount point) is one mis-click away from an unusable system.
//! Everything that deletes, moves or renames goes through
//! `ensure_modifiable` first; copying and browsing these places is fine.

use crate::error::FileManagerError;
use crate::filesystem::mounts::{self, MountPoint};
use crate::navigation::locations;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Directories that are part of the operating system's skeleton. Exact
/// matches only -- `/usr/bin/some-tool` is a file inside one, not one of them.
const SYSTEM_DIRS: &[&str] = &[
    "/",
    "/bin",
    "/boot",
    "/dev",
    "/etc",
    "/home",
    "/lib",
    "/lib32",
    "/lib64",
    "/libx32",
    "/lost+found",
    "/media",
    "/mnt",
    "/opt",
    "/proc",
    "/root",
    "/run",
    "/run/media",
    "/run/user",
    "/sbin",
    "/srv",
    "/sys",
    "/tmp",
    "/usr",
    "/usr/bin",
    "/usr/lib",
    "/usr/lib64",
    "/usr/libexec",
    "/usr/local",
    "/usr/sbin",
    "/usr/share",
    "/var",
    "/var/cache",
    "/var/lib",
    "/var/log",
    "/var/tmp",
];

/// Whole trees that only an administrator normally writes to.
const SYSTEM_AREAS: &[&str] = &[
    "/bin", "/boot", "/dev", "/etc", "/lib", "/lib32", "/lib64", "/libx32", "/proc", "/root",
    "/sbin", "/sys", "/usr", "/var",
];

/// Inside `SYSTEM_AREAS` but ordinary, user-writable scratch space.
const SYSTEM_AREA_EXCEPTIONS: &[&str] = &["/var/tmp"];

/// Why `path` must not be deleted, moved or renamed -- or `None` if it may
/// be. `mounts` is the current mount table (a parameter so tests can supply
/// one).
fn protected_reason_with(path: &Path, mounts: &[MountPoint]) -> Option<&'static str> {
    let forms = comparable_forms(path);

    if forms
        .iter()
        .any(|form| SYSTEM_DIRS.iter().any(|dir| form == Path::new(dir)))
    {
        return Some("is a protected system folder");
    }

    if forms.iter().any(|form| is_personal_folder(form)) {
        return Some("is one of your personal folders");
    }

    if forms
        .iter()
        .any(|form| mounts.iter().any(|mount| mount.path == *form))
    {
        return Some("is a mount point (unmount it instead)");
    }

    None
}

fn is_personal_folder(path: &Path) -> bool {
    let personal = [
        locations::home_dir(),
        locations::desktop_dir(),
        locations::documents_dir(),
        locations::downloads_dir(),
        locations::music_dir(),
        locations::pictures_dir(),
        locations::videos_dir(),
        locations::public_dir(),
    ];

    personal.iter().any(|dir| dir == path)
}

/// Refuse (with a message the user can act on) if any of `paths` is
/// protected.
pub fn ensure_modifiable(paths: &[PathBuf]) -> Result<(), FileManagerError> {
    let mounts = mounts::list();

    for path in paths {
        if let Some(reason) = protected_reason_with(path, &mounts) {
            return Err(FileManagerError::Protected(format!(
                "\"{}\" {reason}, so it can't be deleted, moved or renamed.",
                path.display()
            )));
        }
    }

    Ok(())
}

/// Is `path` somewhere only an administrator normally changes (`/etc`,
/// `/usr`, `/var`, ...)? Used to warn before destructive operations there
/// and to label such folders in the status bar.
pub fn is_system_area(path: &Path) -> bool {
    comparable_forms(path).iter().any(|form| {
        SYSTEM_AREAS.iter().any(|area| form.starts_with(area))
            && !SYSTEM_AREA_EXCEPTIONS
                .iter()
                .any(|exception| form.starts_with(exception))
    })
}

/// The spellings of `path` worth comparing: the tidied-up path as given,
/// plus -- for anything that isn't itself a symlink -- its fully resolved
/// form, so `/usr/../etc` and a bind-mounted alias are still recognised.
///
/// A symlink is judged by where the *link* sits, never by what it points
/// at: deleting `~/shortcut-to-etc` removes a link, not `/etc`.
fn comparable_forms(path: &Path) -> Vec<PathBuf> {
    let lexical = tidy(path);
    let mut forms = vec![lexical.clone()];

    let is_symlink = fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);

    if !is_symlink {
        if let Ok(resolved) = fs::canonicalize(path) {
            if resolved != lexical {
                forms.push(resolved);
            }
        }
    }

    forms
}

/// Resolve `.` and `..` by text alone (the path may not exist), and drop
/// trailing slashes.
fn tidy(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if result.parent().is_some() {
                    result.pop();
                } else if !result.has_root() {
                    // A relative path that climbs above where it started.
                    result.push(component.as_os_str());
                }
                // Otherwise `..` at the root stays at the root.
            }
            other => result.push(other.as_os_str()),
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn system_folders_are_protected_but_files_inside_them_are_not() {
        let none: &[MountPoint] = &[];

        assert!(protected_reason_with(Path::new("/"), none).is_some());
        assert!(protected_reason_with(Path::new("/usr"), none).is_some());
        assert!(protected_reason_with(Path::new("/usr/"), none).is_some());
        assert!(protected_reason_with(Path::new("/usr/../etc"), none).is_some());
        assert!(protected_reason_with(Path::new("/usr/bin/env"), none).is_none());
        assert!(protected_reason_with(Path::new("/etc/hostname"), none).is_none());
    }

    #[test]
    fn mount_points_are_protected() {
        let mounts = mounts::parse("/dev/sdb1 /media/user/STICK vfat rw 0 0\n");

        assert!(protected_reason_with(Path::new("/media/user/STICK"), &mounts).is_some());
        assert!(protected_reason_with(Path::new("/media/user/STICK/photo.jpg"), &mounts).is_none());
    }

    #[test]
    fn a_symlink_is_judged_by_where_it_sits_not_where_it_points() {
        let dir = scratch_dir("protect-symlink");
        let link = dir.join("shortcut-to-etc");
        std::os::unix::fs::symlink("/etc", &link).unwrap();

        assert!(protected_reason_with(&link, &[]).is_none());
        assert!(protected_reason_with(Path::new("/etc"), &[]).is_some());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ordinary_scratch_paths_are_fine() {
        let dir = scratch_dir("protect-ordinary");

        assert!(ensure_modifiable(&[dir.join("anything")]).is_ok());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_error_names_the_offending_path() {
        let err = ensure_modifiable(&[PathBuf::from("/usr")]).unwrap_err();

        assert!(err.to_string().contains("/usr"));
        assert!(matches!(err, FileManagerError::Protected(_)));
    }

    #[test]
    fn system_areas_exclude_user_scratch_space() {
        assert!(is_system_area(Path::new("/etc/passwd")));
        assert!(is_system_area(Path::new("/usr/share/doc")));
        assert!(is_system_area(Path::new("/var/log/syslog")));
        assert!(!is_system_area(Path::new("/var/tmp/scratch")));
        assert!(!is_system_area(Path::new("/home/someone/file")));
    }

    #[test]
    fn tidy_resolves_dots_without_touching_the_disk() {
        assert_eq!(tidy(Path::new("/a/b/../c/./d/")), PathBuf::from("/a/c/d"));
        assert_eq!(tidy(Path::new("/../..")), PathBuf::from("/"));
        assert_eq!(tidy(Path::new("a/../../b")), PathBuf::from("../b"));
    }
}
