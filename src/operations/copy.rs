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
        fs::copy(source, destination)?;
    }

    Ok(())
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
            fs::copy(entry.path(), target)?;
        }
    }

    Ok(())
}
