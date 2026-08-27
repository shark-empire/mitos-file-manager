pub mod copy;
pub mod create;
pub mod move_op;
pub mod rename;
pub mod trash;

use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub enum PendingOp {
    Copy,
    Move,
}

pub fn paste_pending(
    destination_dir: &Path,
    operation: PendingOp,
    sources: &[PathBuf],
) -> io::Result<usize> {
    let mut pasted = 0;

    for source in sources {
        let file_name = source.file_name().unwrap_or_default().to_os_string();
        let destination = unique_destination(&destination_dir.join(file_name));

        match operation {
            PendingOp::Copy => copy::copy_path(source, &destination)?,
            PendingOp::Move => move_op::move_path(source, &destination)?,
        }

        pasted += 1;
    }

    Ok(pasted)
}

pub fn unique_destination(destination: &Path) -> PathBuf {
    if !destination.exists() {
        return destination.to_path_buf();
    }

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));

    let file_name = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let (stem, extension): (String, Option<String>) =
        if file_name.starts_with('.') && file_name.matches('.').count() == 1 {
            (file_name.clone(), None)
        } else {
            match file_name.rsplit_once('.') {
                Some((stem, extension)) => (stem.to_string(), Some(extension.to_string())),
                None => (file_name.clone(), None),
            }
        };

    let mut counter = 1;

    loop {
        let candidate = match &extension {
            Some(extension) => parent.join(format!("{stem} ({counter}).{extension}")),
            None => parent.join(format!("{stem} ({counter})")),
        };

        if !candidate.exists() {
            return candidate;
        }

        counter += 1;
    }
}
