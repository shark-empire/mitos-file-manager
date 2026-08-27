use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn create_folder(parent: &Path, name: &str) -> io::Result<PathBuf> {
    let path = parent.join(name);
    fs::create_dir_all(&path)?;
    Ok(path)
}

pub fn create_file(parent: &Path, name: &str) -> io::Result<PathBuf> {
    let path = parent.join(name);
    fs::File::create(&path)?;
    Ok(path)
}
