use crate::operations::copy::copy_path;
use std::fs;
use std::io;
use std::path::Path;

pub fn move_path(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(_) => Ok(()),
        Err(err) => {
            const EXDEV: i32 = 18;

            if err.raw_os_error() == Some(EXDEV) {
                copy_path(source, destination)?;
                remove_all(source)?;
                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

fn remove_all(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}
