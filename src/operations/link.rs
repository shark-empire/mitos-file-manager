use crate::error::FileManagerError;
use crate::operations::unique_destination;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// Create a symbolic link to `target` inside `directory`, named
/// "Link to <name>" (or "Link to <name> (1)", ... if that's taken), and
/// return the new link's path.
///
/// The link stores an absolute path, so it keeps working if the link itself
/// is moved somewhere else afterwards.
pub fn create_symlink(target: &Path, directory: &Path) -> Result<PathBuf, FileManagerError> {
    if !directory.is_dir() {
        return Err(FileManagerError::NotADirectory);
    }

    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or(FileManagerError::InvalidName)?;

    let link_path = unique_destination(&directory.join(format!("Link to {name}")));

    let absolute = if target.is_absolute() {
        target.to_path_buf()
    } else {
        std::env::current_dir()?.join(target)
    };

    symlink(&absolute, &link_path)?;

    Ok(link_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;
    use std::fs;

    #[test]
    fn creates_a_named_link_pointing_at_the_target() {
        let dir = scratch_dir("link-basic");
        let target = dir.join("report.txt");
        fs::write(&target, "data").unwrap();

        let link = create_symlink(&target, &dir).unwrap();

        assert_eq!(link, dir.join("Link to report.txt"));
        assert_eq!(fs::read_link(&link).unwrap(), target);
        assert_eq!(fs::read_to_string(&link).unwrap(), "data");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_link_gets_a_free_name_instead_of_failing() {
        let dir = scratch_dir("link-twice");
        let target = dir.join("a");
        fs::write(&target, "x").unwrap();

        let first = create_symlink(&target, &dir).unwrap();
        let second = create_symlink(&target, &dir).unwrap();

        assert_ne!(first, second);
        assert_eq!(second, dir.join("Link to a (1)"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn links_can_point_at_folders_and_go_only_into_folders() {
        let dir = scratch_dir("link-folder");
        let folder = dir.join("stuff");
        fs::create_dir(&folder).unwrap();

        let link = create_symlink(&folder, &dir).unwrap();
        assert!(link.is_dir());

        assert!(matches!(
            create_symlink(&folder, &dir.join("missing")),
            Err(FileManagerError::NotADirectory)
        ));

        let _ = fs::remove_dir_all(&dir);
    }
}
