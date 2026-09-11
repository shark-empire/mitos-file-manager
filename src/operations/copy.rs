use mitos_utils::applets::cp;
use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_unix;

pub fn copy_path(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;

    if metadata.is_dir() {
        copy_dir_all(source, destination)?;
    } else if metadata.file_type().is_symlink() {
        let target = fs::read_link(source)?;

        #[cfg(unix)]
        symlink_unix(target, destination)?;

        #[cfg(not(unix))]
        {
            let _ = target;
        }
    } else {
        copy_file(source, destination).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    }

    Ok(())
}

/// Copy a single regular file through the same `mitos-utils` `cp` logic
/// used by the `mitos-cp` terminal command, so GUI and CLI copies behave
/// identically. Directory recursion and symlink handling stay in
/// `copy_path`/`copy_dir_all`; this only performs the leaf-level file copy.
pub fn copy_file(source: &Path, dest: &Path) -> Result<(), String> {
    // This calls the exact same logic as running "mitos-cp" in the terminal
    // args: ["cp", "-p", source_str, dest_str] (preserve timestamps)
    let args = vec![
        "cp".to_string(),
        "-p".to_string(),
        source.to_string_lossy().to_string(),
        dest.to_string_lossy().to_string(),
    ];

    cp::run(args).map_err(|e| e.to_string())
}

fn copy_dir_all(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else if file_type.is_symlink() {
            let link_target = fs::read_link(entry.path())?;

            #[cfg(unix)]
            symlink_unix(link_target, target)?;

            #[cfg(not(unix))]
            {
                let _ = link_target;
            }
        } else {
            let entry_path = entry.path();
            copy_file(&entry_path, &target)
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        }
    }

    Ok(())
}
