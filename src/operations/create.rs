use crate::error::FileManagerError;
use crate::operations::validate_name;
use std::fs;
use std::path::{Path, PathBuf};

pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf, FileManagerError> {
    let name = validate_name(name)?;

    if !parent.is_dir() {
        return Err(FileManagerError::NotADirectory);
    }

    let path = parent.join(name);

    // `create_dir`, not `create_dir_all`: an existing folder is reported as
    // "already exists" instead of being silently "created" again, and a
    // name can't smuggle in extra path components.
    fs::create_dir(&path)?;

    Ok(path)
}

pub fn create_file(parent: &Path, name: &str) -> Result<PathBuf, FileManagerError> {
    let name = validate_name(name)?;

    if !parent.is_dir() {
        return Err(FileManagerError::NotADirectory);
    }

    let path = parent.join(name);

    // `File::create` would truncate an existing file to zero bytes.
    // `create_new` fails with AlreadyExists instead, so "New File" can never
    // wipe out something that's already there.
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn new_file_never_truncates_an_existing_one() {
        let dir = scratch_dir("create-file");

        let path = create_file(&dir, "note.txt").unwrap();
        fs::write(&path, "keep me").unwrap();

        assert!(create_file(&dir, "note.txt").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_folder_rejects_bad_names_missing_parents_and_duplicates() {
        let dir = scratch_dir("create-folder");

        assert!(matches!(
            create_folder(&dir, "a/b"),
            Err(FileManagerError::InvalidName)
        ));
        assert!(matches!(
            create_folder(&dir, ".."),
            Err(FileManagerError::InvalidName)
        ));
        assert!(matches!(
            create_folder(&dir.join("missing"), "x"),
            Err(FileManagerError::NotADirectory)
        ));

        assert!(create_folder(&dir, "sub").unwrap().is_dir());
        assert!(create_folder(&dir, "sub").is_err());

        let _ = fs::remove_dir_all(&dir);
    }
}
