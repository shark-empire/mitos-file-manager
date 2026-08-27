use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn rename_path(source: &Path, new_name: &str) -> io::Result<PathBuf> {
    let destination = source.with_file_name(new_name);
    fs::rename(source, &destination)?;
    Ok(destination)
}
